// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Quince trading engine — event loop, order manager, indicator bank.
//!
//! The [`Engine`](r#loop::Engine) drives the strategy lifecycle: feeds market
//! data into the QFL runtime, dispatches orders, manages hot-reload, and
//! coordinates with the exchange connector.

pub mod control;
pub mod indicators;
pub mod journal;
pub mod r#loop;
pub mod orders;
pub mod strategy_lifecycle;
pub mod telemetry;

pub use control::{
    default_strategy_control_channel, strategy_control_channel, StrategyControlAuditRecord,
    StrategyControlAuditStatus, StrategyControlCommand, StrategyControlCommandKind,
    StrategyControlError, StrategyControlReceiver, StrategyControlRequest, StrategyControlSender,
    DEFAULT_CONTROL_AUDIT_CAPACITY, DEFAULT_CONTROL_QUEUE_CAPACITY,
};
pub use journal::{JournalEvent, JournalRecord, OrderJournal};
pub use r#loop::{Engine, EngineError};
pub use strategy_lifecycle::{
    DeploymentMode, StrategyLifecycle, StrategyLifecycleError, StrategyRevision, StrategySlot,
};
pub use telemetry::{RuntimeTelemetry, RuntimeTelemetrySnapshot};
