// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Risk control enforcement at runtime.
//! [`RiskControls`] validates orders and positions against configured limits
//! (max position size, max drawdown, order frequency, daily loss, cooldown).

use quince_core::types::*;
use std::time::{Duration, Instant};

const DEFAULT_MAX_MARKET_DATA_AGE: Duration = Duration::from_secs(5);

pub struct RiskControls {
    pub max_position_size: f64,
    pub max_drawdown: f64,
    pub max_order_freq: u32,
    pub max_daily_loss: f64,
    pub cooldown_after_loss_secs: u64,

    pub(super) order_count: u32,
    pub(super) window_start: Instant,
    pub(super) daily_loss: f64,
    pub(super) peak_equity: f64,
    pub(super) in_cooldown: bool,
    pub(super) cooldown_end: Instant,
    last_market_data_at: Option<Instant>,
    max_market_data_age: Duration,
    paused: bool,
    pause_reason: Option<String>,
}

impl RiskControls {
    pub fn new(config: crate::RiskConfig) -> Self {
        Self {
            max_position_size: config.max_position_size,
            max_drawdown: config.max_drawdown,
            max_order_freq: config.max_order_freq,
            max_daily_loss: config.max_daily_loss,
            cooldown_after_loss_secs: config.cooldown_after_loss_secs,
            order_count: 0,
            window_start: Instant::now(),
            daily_loss: 0.0,
            peak_equity: 0.0,
            in_cooldown: false,
            cooldown_end: Instant::now(),
            last_market_data_at: None,
            max_market_data_age: DEFAULT_MAX_MARKET_DATA_AGE,
            paused: false,
            pause_reason: None,
        }
    }

    /// Stop new orders until an operator explicitly resumes execution.
    ///
    /// This is intentionally sticky: a transient recovery in an account or
    /// price feed must not silently re-enable trading after a safety trip.
    pub fn pause(&mut self, reason: impl Into<String>) {
        self.paused = true;
        self.pause_reason = Some(reason.into());
    }

    /// Explicit operator acknowledgement for a previously paused strategy.
    pub fn resume(&mut self) {
        self.paused = false;
        self.pause_reason = None;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Records a fresh trade, mark-price, or order-book observation.
    ///
    /// A later call to [`Self::check_order`] requires such an observation to
    /// be recent. This prevents an evaluation timer from trading on a frozen
    /// market-data stream.
    pub fn record_market_data(&mut self) {
        self.last_market_data_at = Some(Instant::now());
    }

    /// Sets the maximum permitted market-data age before new execution is
    /// stopped. A zero duration deliberately makes the guard fail closed as
    /// soon as the monotonic clock advances; callers should normally use a
    /// small positive duration.
    pub fn set_max_market_data_age(&mut self, max_age: Duration) {
        self.max_market_data_age = max_age;
    }

    fn reject_and_pause(&mut self, reason: impl Into<String>) -> Result<(), String> {
        let reason = reason.into();
        self.pause(reason.clone());
        Err(reason)
    }

    fn check_market_data_freshness(&mut self) -> Result<(), String> {
        let Some(observed_at) = self.last_market_data_at else {
            return self.reject_and_pause("market data has not been observed");
        };
        let now = Instant::now();
        let Some(age) = now.checked_duration_since(observed_at) else {
            return self.reject_and_pause("market data timestamp is invalid");
        };
        if age > self.max_market_data_age {
            return self.reject_and_pause(format!(
                "market data is stale: age {} ms exceeds limit {} ms",
                age.as_millis(),
                self.max_market_data_age.as_millis()
            ));
        }
        Ok(())
    }

    pub fn check_order(
        &mut self,
        order: &Order,
        current_equity: f64,
        current_position: f64,
    ) -> Result<(), String> {
        if self.paused {
            return Err(format!(
                "risk execution is paused: {}",
                self.pause_reason
                    .as_deref()
                    .unwrap_or("operator action required")
            ));
        }

        self.check_market_data_freshness()?;

        if self.in_cooldown {
            if Instant::now() < self.cooldown_end {
                return Err("in cooldown after loss".into());
            }
            self.in_cooldown = false;
        }

        if !order.qty.is_finite() || order.qty <= 0.0 {
            return Err("order qty must be finite and positive".into());
        }
        if !current_equity.is_finite() {
            return self.reject_and_pause("current equity must be finite");
        }
        if !current_position.is_finite() {
            return self.reject_and_pause("current position must be finite");
        }
        let signed_qty = match order.side {
            Side::Buy => order.qty,
            Side::Sell => -order.qty,
        };
        if order.reduce_only {
            if current_position == 0.0
                || signed_qty.signum() == current_position.signum()
                || order.qty > current_position.abs()
            {
                return Err("reduce-only order would not reduce the current position".into());
            }
        } else if (current_position + signed_qty).abs() > self.max_position_size {
            return Err(format!(
                "position {} would exceed max position size {}",
                (current_position + signed_qty).abs(),
                self.max_position_size
            ));
        }

        self.peak_equity = self.peak_equity.max(current_equity);
        if self.peak_equity > 0.0 {
            let drawdown = (self.peak_equity - current_equity) / self.peak_equity;
            if drawdown > self.max_drawdown {
                return self.reject_and_pause(format!(
                    "drawdown {:.2}% exceeds limit {:.2}%",
                    drawdown * 100.0,
                    self.max_drawdown * 100.0
                ));
            }
        }

        if self.daily_loss >= self.max_daily_loss {
            return self.reject_and_pause(format!(
                "daily loss {:.2} exceeds limit {:.2}",
                self.daily_loss, self.max_daily_loss
            ));
        }

        let elapsed = Instant::now().duration_since(self.window_start);
        if elapsed < Duration::from_secs(1) && self.order_count >= self.max_order_freq {
            return Err("rate limit exceeded".into());
        }

        if elapsed > Duration::from_secs(1) {
            self.window_start = Instant::now();
            self.order_count = 0;
        }

        Ok(())
    }

    pub fn record_trade(&mut self) {
        self.order_count += 1;
    }

    pub fn record_loss(&mut self, loss: f64) {
        self.daily_loss += loss;
        if loss > 0.0 {
            self.in_cooldown = true;
            self.cooldown_end = Instant::now() + Duration::from_secs(self.cooldown_after_loss_secs);
        }
    }

    pub fn reset_daily(&mut self) {
        self.daily_loss = 0.0;
        self.order_count = 0;
        self.window_start = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::types::{Order, OrderType, Side};

    fn make_order(qty: f64) -> Order {
        Order {
            symbol: "btcusdt".into(),
            side: Side::Buy,
            qty,
            price: None,
            order_type: OrderType::Market,
            reduce_only: false,
            stop_loss: None,
            take_profit: None,
        }
    }

    fn risk() -> RiskControls {
        let mut controls = RiskControls::new(crate::RiskConfig {
            max_position_size: 10.0,
            max_drawdown: 0.1,
            max_order_freq: 5,
            max_daily_loss: 1000.0,
            cooldown_after_loss_secs: 0,
        });
        controls.record_market_data();
        controls
    }

    #[test]
    fn check_order_ok() {
        let mut r = risk();
        assert!(r.check_order(&make_order(1.0), 10000.0, 0.0).is_ok());
    }

    #[test]
    fn check_order_exceeds_max_position() {
        let mut r = risk();
        let result = r.check_order(&make_order(20.0), 10000.0, 0.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("max position size"));
    }

    #[test]
    fn check_order_rejects_cumulative_position_limit_breach() {
        let mut r = risk();
        let result = r.check_order(&make_order(2.0), 10000.0, 9.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("position 11"));
    }

    #[test]
    fn check_order_drawdown_exceeded() {
        let mut r = risk();
        // peak_equity 10000, current equity 8000 в†’ drawdown 20% > 10%
        assert!(r.check_order(&make_order(1.0), 10000.0, 0.0).is_ok());
        let result = r.check_order(&make_order(1.0), 8000.0, 0.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("drawdown"));
    }

    #[test]
    fn check_order_daily_loss_exceeded() {
        let mut r = risk();
        r.record_loss(1500.0);
        let result = r.check_order(&make_order(1.0), 10000.0, 0.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("daily loss"));
    }

    #[test]
    fn check_order_rate_limit_exceeded() {
        let mut r = risk();
        r.window_start = Instant::now();
        for _ in 0..5 {
            assert!(r.check_order(&make_order(1.0), 10000.0, 0.0).is_ok());
            r.record_trade();
        }
        let result = r.check_order(&make_order(1.0), 10000.0, 0.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("rate limit"));
    }

    #[test]
    fn check_cooldown_rejects_orders() {
        let mut r = risk();
        r.in_cooldown = true;
        r.cooldown_end = Instant::now() + Duration::from_secs(3600);
        let result = r.check_order(&make_order(1.0), 10000.0, 0.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cooldown"));
    }

    #[test]
    fn cooldown_expires() {
        let mut r = risk();
        r.in_cooldown = true;
        r.cooldown_end = Instant::now() - Duration::from_secs(1);
        assert!(r.check_order(&make_order(1.0), 10000.0, 0.0).is_ok());
        assert!(!r.in_cooldown);
    }

    #[test]
    fn record_trade_increments_order_count() {
        let mut r = risk();
        assert_eq!(r.order_count, 0);
        r.record_trade();
        assert_eq!(r.order_count, 1);
        r.record_trade();
        assert_eq!(r.order_count, 2);
    }

    #[test]
    fn record_loss_activates_cooldown() {
        let mut r = risk();
        r.cooldown_after_loss_secs = 60;
        r.record_loss(100.0);
        assert!(r.in_cooldown);
    }

    #[test]
    fn record_loss_zero_does_not_activate_cooldown() {
        let mut r = risk();
        r.record_loss(0.0);
        assert!(!r.in_cooldown);
    }

    #[test]
    fn reset_daily_clears_state() {
        let mut r = risk();
        r.daily_loss = 500.0;
        r.order_count = 10;
        r.reset_daily();
        assert_eq!(r.daily_loss, 0.0);
        assert_eq!(r.order_count, 0);
    }

    #[test]
    fn peak_equity_tracking() {
        let mut r = risk();
        r.check_order(&make_order(1.0), 5000.0, 0.0).ok();
        assert_eq!(r.peak_equity, 5000.0);
        r.check_order(&make_order(1.0), 6000.0, 0.0).ok();
        assert_eq!(r.peak_equity, 6000.0);
        r.check_order(&make_order(1.0), 4000.0, 0.0).ok();
        assert_eq!(r.peak_equity, 6000.0);
    }

    #[test]
    fn zero_peak_equity_skips_drawdown() {
        let mut r = risk();
        assert!(r.check_order(&make_order(1.0), 0.0, 0.0).is_ok());
    }

    #[test]
    fn rate_limit_window_resets_after_one_sec() {
        let mut r = risk();
        r.window_start = Instant::now() - Duration::from_secs(2);
        r.order_count = 10;
        assert!(r.check_order(&make_order(1.0), 10000.0, 0.0).is_ok());
        assert_eq!(r.order_count, 0);
        r.record_trade();
        assert_eq!(r.order_count, 1);
    }

    #[test]
    fn manual_pause_rejects_orders_until_explicitly_resumed() {
        let mut r = risk();
        r.pause("operator intervention");

        let error = r
            .check_order(&make_order(1.0), 10_000.0, 0.0)
            .expect_err("paused execution must fail closed");
        assert!(error.contains("operator intervention"));
        assert!(r.is_paused());

        r.resume();
        assert!(r.check_order(&make_order(1.0), 10_000.0, 0.0).is_ok());
    }

    #[test]
    fn drawdown_breach_latches_kill_switch_until_resumed() {
        let mut r = risk();
        assert!(r.check_order(&make_order(1.0), 10_000.0, 0.0).is_ok());

        let error = r
            .check_order(&make_order(1.0), 8_000.0, 0.0)
            .expect_err("drawdown must stop new risk");
        assert!(error.contains("drawdown"));
        assert!(r.is_paused());

        let error = r
            .check_order(&make_order(1.0), 10_000.0, 0.0)
            .expect_err("a recovered price must not silently re-enable trading");
        assert!(error.contains("drawdown"));
    }

    #[test]
    fn non_finite_equity_pauses_execution() {
        let mut r = risk();
        let error = r
            .check_order(&make_order(1.0), f64::NAN, 0.0)
            .expect_err("unknown equity must fail closed");
        assert!(error.contains("equity"));
        assert!(r.is_paused());
    }

    #[test]
    fn daily_loss_limit_is_inclusive_and_latched() {
        let mut r = risk();
        r.record_loss(1_000.0);
        let error = r
            .check_order(&make_order(1.0), 10_000.0, 0.0)
            .expect_err("loss at the configured ceiling must stop trading");
        assert!(error.contains("daily loss"));
        assert!(r.is_paused());
    }

    #[test]
    fn missing_market_data_latches_kill_switch() {
        let mut r = RiskControls::new(crate::RiskConfig::default());

        let error = r
            .check_order(&make_order(1.0), 10_000.0, 0.0)
            .expect_err("orders without a market-data observation must fail closed");
        assert!(error.contains("market data has not been observed"));
        assert!(r.is_paused());
    }

    #[test]
    fn stale_market_data_latches_kill_switch_until_operator_resumes() {
        let mut r = risk();
        r.last_market_data_at =
            Some(Instant::now() - r.max_market_data_age - Duration::from_millis(1));

        let error = r
            .check_order(&make_order(1.0), 10_000.0, 0.0)
            .expect_err("stale market data must fail closed");
        assert!(error.contains("market data is stale"));
        assert!(r.is_paused());

        r.record_market_data();
        let error = r
            .check_order(&make_order(1.0), 10_000.0, 0.0)
            .expect_err("a fresh tick cannot silently clear a stale-data kill switch");
        assert!(error.contains("market data is stale"));

        r.resume();
        assert!(r.check_order(&make_order(1.0), 10_000.0, 0.0).is_ok());
    }
}
