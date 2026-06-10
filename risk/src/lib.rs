// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Risk management configuration and controls.
//! Defines [`RiskConfig`] for parameterizing position sizing, drawdown limits,
//! order frequency, daily loss caps, and cooldown periods.

pub mod controls;

pub use controls::RiskControls;

pub struct RiskConfig {
    pub max_position_size: f64,
    pub max_drawdown: f64,
    pub max_order_freq: u32,
    pub max_daily_loss: f64,
    pub cooldown_after_loss_secs: u64,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_position_size: 1.0,
            max_drawdown: 0.05,
            max_order_freq: 10,
            max_daily_loss: 1000.0,
            cooldown_after_loss_secs: 60,
        }
    }
}
