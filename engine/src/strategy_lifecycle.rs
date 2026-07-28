// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Versioned, rollback-safe strategy deployment state.
//!
//! This module deliberately contains no VM or exchange code.  A caller must
//! compile and validate a candidate before calling [`StrategyLifecycle::deploy`];
//! deployment then changes the active slot atomically from the caller's point
//! of view.  The previous slot retains its own opaque runtime state, so a
//! rollback can never run state created by a different strategy version.

use serde::{Deserialize, Serialize};

/// Whether a deployed strategy may emit orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeploymentMode {
    /// Evaluate the strategy and record its decisions, but do not submit orders.
    Shadow,
    /// Normal order-producing deployment.
    Live,
}

/// Immutable identity of compiled strategy code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyRevision {
    /// A monotonically increasing, operator-assigned deployment version.
    pub version: u64,
    /// SHA-256 (or equivalent 32-byte content digest) of the validated artifact.
    pub artifact_digest: [u8; 32],
    pub mode: DeploymentMode,
}

impl StrategyRevision {
    pub fn new(version: u64, artifact_digest: [u8; 32], mode: DeploymentMode) -> Self {
        Self {
            version,
            artifact_digest,
            mode,
        }
    }
}

/// A revision plus only the state generated while that exact revision was active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategySlot {
    pub revision: StrategyRevision,
    /// Opaque, version-scoped runtime checkpoint.  The VM owns its encoding.
    pub runtime_state: Vec<u8>,
}

impl StrategySlot {
    pub fn new(revision: StrategyRevision, runtime_state: Vec<u8>) -> Self {
        Self {
            revision,
            runtime_state,
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StrategyLifecycleError {
    #[error("strategy version must be greater than zero")]
    ZeroVersion,
    #[error("strategy version {candidate} is not newer than active version {active}")]
    NonMonotonicVersion { candidate: u64, active: u64 },
    #[error("no previous strategy revision is available for rollback")]
    NoRollbackTarget,
    #[error("no active strategy revision is deployed")]
    NoActiveRevision,
}

/// Two-slot deployment register.
///
/// At most one live revision and one known-good rollback target are retained.
/// `deploy` validates all invariants before mutating either slot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrategyLifecycle {
    active: Option<StrategySlot>,
    previous: Option<StrategySlot>,
}

impl StrategyLifecycle {
    pub fn active(&self) -> Option<&StrategySlot> {
        self.active.as_ref()
    }

    pub fn rollback_target(&self) -> Option<&StrategySlot> {
        self.previous.as_ref()
    }

    /// Install a validated candidate.  The candidate begins with an empty
    /// checkpoint; it cannot inherit state from the revision it replaces.
    pub fn deploy(&mut self, revision: StrategyRevision) -> Result<(), StrategyLifecycleError> {
        if revision.version == 0 {
            return Err(StrategyLifecycleError::ZeroVersion);
        }
        if let Some(active) = &self.active {
            if revision.version <= active.revision.version {
                return Err(StrategyLifecycleError::NonMonotonicVersion {
                    candidate: revision.version,
                    active: active.revision.version,
                });
            }
        }

        let candidate = StrategySlot::new(revision, Vec::new());
        self.previous = self.active.replace(candidate);
        Ok(())
    }

    /// Persist a checkpoint for the active revision only.
    pub fn checkpoint(&mut self, runtime_state: Vec<u8>) -> Result<(), StrategyLifecycleError> {
        let active = self
            .active
            .as_mut()
            .ok_or(StrategyLifecycleError::NoActiveRevision)?;
        active.runtime_state = runtime_state;
        Ok(())
    }

    /// Swap the complete slots.  Swapping, rather than copying state, makes
    /// rollback reversible and guarantees state/revision pairing.
    pub fn rollback(&mut self) -> Result<(), StrategyLifecycleError> {
        if self.previous.is_none() {
            return Err(StrategyLifecycleError::NoRollbackTarget);
        }
        std::mem::swap(&mut self.active, &mut self.previous);
        Ok(())
    }

    /// Shadow strategies must never pass this gate into execution.
    pub fn execution_enabled(&self) -> bool {
        matches!(
            self.active.as_ref().map(|slot| slot.revision.mode),
            Some(DeploymentMode::Live)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn revision(version: u64, mode: DeploymentMode) -> StrategyRevision {
        StrategyRevision::new(version, [version as u8; 32], mode)
    }

    #[test]
    fn shadow_deployment_cannot_enable_execution() {
        let mut lifecycle = StrategyLifecycle::default();
        lifecycle
            .deploy(revision(1, DeploymentMode::Shadow))
            .unwrap();

        assert!(!lifecycle.execution_enabled());
        assert_eq!(lifecycle.active().unwrap().runtime_state, Vec::<u8>::new());
    }

    #[test]
    fn deployment_rejects_non_monotonic_version_without_mutating_slots() {
        let mut lifecycle = StrategyLifecycle::default();
        lifecycle.deploy(revision(2, DeploymentMode::Live)).unwrap();
        lifecycle.checkpoint(vec![7, 8]).unwrap();

        let error = lifecycle
            .deploy(revision(2, DeploymentMode::Shadow))
            .unwrap_err();
        assert_eq!(
            error,
            StrategyLifecycleError::NonMonotonicVersion {
                candidate: 2,
                active: 2
            }
        );
        assert_eq!(lifecycle.active().unwrap().runtime_state, vec![7, 8]);
        assert!(lifecycle.rollback_target().is_none());
    }

    #[test]
    fn rollback_restores_matching_state_and_is_reversible() {
        let mut lifecycle = StrategyLifecycle::default();
        lifecycle.deploy(revision(1, DeploymentMode::Live)).unwrap();
        lifecycle.checkpoint(vec![1]).unwrap();
        lifecycle
            .deploy(revision(2, DeploymentMode::Shadow))
            .unwrap();
        lifecycle.checkpoint(vec![2]).unwrap();

        lifecycle.rollback().unwrap();
        assert_eq!(lifecycle.active().unwrap().revision.version, 1);
        assert_eq!(lifecycle.active().unwrap().runtime_state, vec![1]);
        assert!(lifecycle.execution_enabled());

        lifecycle.rollback().unwrap();
        assert_eq!(lifecycle.active().unwrap().revision.version, 2);
        assert_eq!(lifecycle.active().unwrap().runtime_state, vec![2]);
        assert!(!lifecycle.execution_enabled());
    }

    #[test]
    fn first_deployment_must_use_nonzero_version() {
        let mut lifecycle = StrategyLifecycle::default();
        assert_eq!(
            lifecycle.deploy(revision(0, DeploymentMode::Live)),
            Err(StrategyLifecycleError::ZeroVersion)
        );
        assert!(lifecycle.active().is_none());
    }

    #[test]
    fn checkpoint_requires_an_active_revision() {
        assert_eq!(
            StrategyLifecycle::default().checkpoint(vec![1]),
            Err(StrategyLifecycleError::NoActiveRevision)
        );
    }
}
