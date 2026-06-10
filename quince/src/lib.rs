// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Quince trading bot — crate root.
//! Re-exports all sub-crates (`core`, `engine`, `exchange`, `indicators`,
//! `logger`, `risk`) as a unified public API for the binary entry point.

pub use quince_core as core;
pub use quince_engine as engine;
pub use quince_exchange as exchange;
pub use quince_indicators as indicators;
pub use quince_logger as logger;
pub use quince_risk as risk;
