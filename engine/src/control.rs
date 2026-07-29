// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Bounded, auditable control-plane commands for strategy lifecycle changes.
//!
//! HTTP and other operator transports only receive a [`StrategyControlSender`].
//! The engine loop owns the matching [`StrategyControlReceiver`] and applies
//! commands through [`crate::StrategyLifecycle`].  This deliberately prevents
//! a transport handler from mutating the VM, journal, or exchange directly.

use chrono::{DateTime, Utc};
use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Default maximum number of commands waiting for the engine loop.
pub const DEFAULT_CONTROL_QUEUE_CAPACITY: usize = 64;
/// Default number of in-memory audit records retained for operator inspection.
pub const DEFAULT_CONTROL_AUDIT_CAPACITY: usize = 1_024;
const MAX_ACTOR_LENGTH: usize = 128;

/// A lifecycle command that an external control plane may request.
///
/// There is intentionally no `DeployLive` or generic `SetMode(Live)` command:
/// an operator must deploy a candidate into shadow and explicitly promote the
/// active shadow revision through the lifecycle state machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum StrategyControlCommand {
    /// Install validated strategy bytes as a fresh shadow revision.
    DeployShadow {
        version: u64,
        artifact_digest: [u8; 32],
    },
    /// Promote the currently active shadow revision to live execution.
    PromoteShadow,
    /// Return to the retained known-good lifecycle slot.
    Rollback,
    /// Demote the active live revision to shadow without allowing a direct
    /// transport-layer mutation of execution state.
    DemoteToShadow,
    /// Latch execution closed immediately through the risk gate.
    PauseExecution { reason: String },
    /// Explicit operator acknowledgement for a prior pause.
    ResumeExecution,
}

/// Stable command label suitable for audit/filtering APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyControlCommandKind {
    DeployShadow,
    PromoteShadow,
    Rollback,
    DemoteToShadow,
    PauseExecution,
    ResumeExecution,
}

impl StrategyControlCommand {
    pub fn kind(&self) -> StrategyControlCommandKind {
        match self {
            Self::DeployShadow { .. } => StrategyControlCommandKind::DeployShadow,
            Self::PromoteShadow => StrategyControlCommandKind::PromoteShadow,
            Self::Rollback => StrategyControlCommandKind::Rollback,
            Self::DemoteToShadow => StrategyControlCommandKind::DemoteToShadow,
            Self::PauseExecution { .. } => StrategyControlCommandKind::PauseExecution,
            Self::ResumeExecution => StrategyControlCommandKind::ResumeExecution,
        }
    }
}

/// A single command with a caller-supplied operator identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyControlRequest {
    pub id: u64,
    pub requested_by: String,
    pub command: StrategyControlCommand,
}

/// Lifecycle command result as retained in the audit stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyControlAuditStatus {
    Queued,
    Applied,
    Rejected,
}

/// Immutable audit event emitted when a command is queued or resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyControlAuditRecord {
    pub audit_sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub request: StrategyControlRequest,
    pub status: StrategyControlAuditStatus,
    /// Machine-readable failure detail. `None` only means successful queueing
    /// or application; it is never used to hide an error.
    pub detail: Option<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StrategyControlError {
    #[error("control queue capacity must be greater than zero")]
    ZeroQueueCapacity,
    #[error("control audit capacity must be greater than zero")]
    ZeroAuditCapacity,
    #[error("control command actor is empty or exceeds {MAX_ACTOR_LENGTH} bytes")]
    InvalidActor,
    #[error("control command queue is full")]
    QueueFull,
    #[error("control command receiver is disconnected")]
    Disconnected,
}

#[derive(Debug)]
struct AuditLog {
    capacity: usize,
    next_sequence: u64,
    records: VecDeque<StrategyControlAuditRecord>,
}

impl AuditLog {
    fn push(
        &mut self,
        request: StrategyControlRequest,
        status: StrategyControlAuditStatus,
        detail: Option<String>,
    ) {
        if self.records.len() == self.capacity {
            self.records.pop_front();
        }
        let record = StrategyControlAuditRecord {
            audit_sequence: self.next_sequence,
            timestamp: Utc::now(),
            request,
            status,
            detail,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.records.push_back(record);
    }
}

/// Send-only side exposed to control-plane transports.
#[derive(Debug, Clone)]
pub struct StrategyControlSender {
    sender: Sender<StrategyControlRequest>,
    next_request_id: Arc<AtomicU64>,
    audit: Arc<Mutex<AuditLog>>,
}

impl StrategyControlSender {
    /// Attempt to queue a command without blocking a web handler or caller.
    pub fn try_submit(
        &self,
        requested_by: impl Into<String>,
        command: StrategyControlCommand,
    ) -> Result<u64, StrategyControlError> {
        let requested_by = requested_by.into();
        let id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let request = StrategyControlRequest {
            id,
            requested_by,
            command,
        };
        if request.requested_by.trim().is_empty() || request.requested_by.len() > MAX_ACTOR_LENGTH {
            self.record(
                request,
                StrategyControlAuditStatus::Rejected,
                Some("invalid requested_by".into()),
            );
            return Err(StrategyControlError::InvalidActor);
        }

        match self.sender.try_send(request.clone()) {
            Ok(()) => {
                self.record(request, StrategyControlAuditStatus::Queued, None);
                Ok(id)
            }
            Err(TrySendError::Full(request)) => {
                self.record(
                    request,
                    StrategyControlAuditStatus::Rejected,
                    Some("control command queue is full".into()),
                );
                Err(StrategyControlError::QueueFull)
            }
            Err(TrySendError::Disconnected(request)) => {
                self.record(
                    request,
                    StrategyControlAuditStatus::Rejected,
                    Some("control command receiver is disconnected".into()),
                );
                Err(StrategyControlError::Disconnected)
            }
        }
    }

    fn record(
        &self,
        request: StrategyControlRequest,
        status: StrategyControlAuditStatus,
        detail: Option<String>,
    ) {
        self.audit
            .lock()
            .expect("control audit mutex poisoned")
            .push(request, status, detail);
    }
}

/// Engine-owned receive side. Only this side may take a command from the queue
/// and append its terminal audit result.
#[derive(Debug)]
pub struct StrategyControlReceiver {
    receiver: Receiver<StrategyControlRequest>,
    audit: Arc<Mutex<AuditLog>>,
}

impl StrategyControlReceiver {
    pub fn try_recv(&self) -> Result<StrategyControlRequest, TryRecvError> {
        self.receiver.try_recv()
    }

    /// Record the terminal outcome after the engine applied a command through
    /// the lifecycle state machine.
    pub fn record_outcome(&self, request: StrategyControlRequest, result: Result<(), String>) {
        let (status, detail) = match result {
            Ok(()) => (StrategyControlAuditStatus::Applied, None),
            Err(reason) => (StrategyControlAuditStatus::Rejected, Some(reason)),
        };
        self.audit
            .lock()
            .expect("control audit mutex poisoned")
            .push(request, status, detail);
    }

    /// Snapshot the bounded in-memory audit stream in oldest-first order.
    pub fn audit_records(&self) -> Vec<StrategyControlAuditRecord> {
        self.audit
            .lock()
            .expect("control audit mutex poisoned")
            .records
            .iter()
            .cloned()
            .collect()
    }
}

/// Create a bounded control command queue and bounded audit stream.
pub fn strategy_control_channel(
    queue_capacity: usize,
    audit_capacity: usize,
) -> Result<(StrategyControlSender, StrategyControlReceiver), StrategyControlError> {
    if queue_capacity == 0 {
        return Err(StrategyControlError::ZeroQueueCapacity);
    }
    if audit_capacity == 0 {
        return Err(StrategyControlError::ZeroAuditCapacity);
    }
    let (sender, receiver) = crossbeam_channel::bounded(queue_capacity);
    let audit = Arc::new(Mutex::new(AuditLog {
        capacity: audit_capacity,
        next_sequence: 1,
        records: VecDeque::with_capacity(audit_capacity),
    }));
    Ok((
        StrategyControlSender {
            sender,
            next_request_id: Arc::new(AtomicU64::new(1)),
            audit: Arc::clone(&audit),
        },
        StrategyControlReceiver { receiver, audit },
    ))
}

/// Create a control queue with production defaults.
pub fn default_strategy_control_channel() -> (StrategyControlSender, StrategyControlReceiver) {
    strategy_control_channel(
        DEFAULT_CONTROL_QUEUE_CAPACITY,
        DEFAULT_CONTROL_AUDIT_CAPACITY,
    )
    .expect("nonzero control-plane defaults")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_is_bounded_and_rejection_is_audited() {
        let (sender, receiver) = strategy_control_channel(1, 8).unwrap();
        sender
            .try_submit("operator", StrategyControlCommand::PromoteShadow)
            .unwrap();
        assert_eq!(
            sender.try_submit("operator", StrategyControlCommand::Rollback),
            Err(StrategyControlError::QueueFull)
        );

        let records = receiver.audit_records();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].status, StrategyControlAuditStatus::Queued);
        assert_eq!(records[1].status, StrategyControlAuditStatus::Rejected);
        assert_eq!(
            records[1].detail.as_deref(),
            Some("control command queue is full")
        );
    }

    #[test]
    fn successful_and_failed_engine_outcomes_are_immutable_audit_events() {
        let (sender, receiver) = strategy_control_channel(2, 8).unwrap();
        sender
            .try_submit("alice", StrategyControlCommand::PromoteShadow)
            .unwrap();
        sender
            .try_submit("bob", StrategyControlCommand::Rollback)
            .unwrap();
        let promoted = receiver.try_recv().unwrap();
        let rollback = receiver.try_recv().unwrap();
        receiver.record_outcome(promoted.clone(), Ok(()));
        receiver.record_outcome(rollback.clone(), Err("no rollback target".into()));

        let records = receiver.audit_records();
        assert_eq!(records.len(), 4);
        assert_eq!(records[2].request, promoted);
        assert_eq!(records[2].status, StrategyControlAuditStatus::Applied);
        assert_eq!(records[3].request, rollback);
        assert_eq!(records[3].status, StrategyControlAuditStatus::Rejected);
        assert_eq!(records[3].detail.as_deref(), Some("no rollback target"));
        assert!(records
            .windows(2)
            .all(|pair| pair[0].audit_sequence < pair[1].audit_sequence));
    }

    #[test]
    fn audit_retention_is_bounded_oldest_first() {
        let (sender, receiver) = strategy_control_channel(3, 2).unwrap();
        sender
            .try_submit("alice", StrategyControlCommand::PromoteShadow)
            .unwrap();
        let request = receiver.try_recv().unwrap();
        receiver.record_outcome(request, Ok(()));
        sender
            .try_submit("bob", StrategyControlCommand::Rollback)
            .unwrap();

        let records = receiver.audit_records();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].request.requested_by, "alice");
        assert_eq!(records[0].status, StrategyControlAuditStatus::Applied);
        assert_eq!(records[1].request.requested_by, "bob");
        assert_eq!(records[1].status, StrategyControlAuditStatus::Queued);
    }

    #[test]
    fn transport_cannot_request_direct_live_deployment() {
        let command = StrategyControlCommand::DeployShadow {
            version: 2,
            artifact_digest: [9; 32],
        };
        assert_eq!(command.kind(), StrategyControlCommandKind::DeployShadow);
        assert!(!serde_json::to_string(&command).unwrap().contains("live"));
    }

    #[test]
    fn invalid_actor_is_rejected_and_audited() {
        let (sender, receiver) = strategy_control_channel(1, 2).unwrap();
        assert_eq!(
            sender.try_submit("  ", StrategyControlCommand::Rollback),
            Err(StrategyControlError::InvalidActor)
        );
        let record = receiver.audit_records().pop().unwrap();
        assert_eq!(record.status, StrategyControlAuditStatus::Rejected);
        assert_eq!(record.detail.as_deref(), Some("invalid requested_by"));
    }
}
