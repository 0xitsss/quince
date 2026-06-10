// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Quince trading engine — event loop, order manager, indicator bank.
//!
//! The [`Engine`](r#loop::Engine) drives the strategy lifecycle: feeds market
//! data into the QFL runtime, dispatches orders, manages hot-reload, and
//! coordinates with the exchange connector.

pub mod indicators;
pub mod r#loop;
pub mod orders;

pub use r#loop::{Engine, EngineError};
