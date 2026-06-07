use quince_core::types::*;
use std::time::{Duration, Instant};

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
        }
    }

    pub fn check_order(&mut self, order: &Order, current_equity: f64) -> Result<(), String> {
        if self.in_cooldown {
            if Instant::now() < self.cooldown_end {
                return Err("in cooldown after loss".into());
            }
            self.in_cooldown = false;
        }

        if order.qty > self.max_position_size {
            return Err(format!(
                "order qty {} exceeds max position size {}",
                order.qty, self.max_position_size
            ));
        }

        self.peak_equity = self.peak_equity.max(current_equity);
        if self.peak_equity > 0.0 {
            let drawdown = (self.peak_equity - current_equity) / self.peak_equity;
            if drawdown > self.max_drawdown {
                return Err(format!(
                    "drawdown {:.2}% exceeds limit {:.2}%",
                    drawdown * 100.0,
                    self.max_drawdown * 100.0
                ));
            }
        }

        if self.daily_loss > self.max_daily_loss {
            return Err(format!(
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
        RiskControls::new(crate::RiskConfig {
            max_position_size: 10.0,
            max_drawdown: 0.1,
            max_order_freq: 5,
            max_daily_loss: 1000.0,
            cooldown_after_loss_secs: 0,
        })
    }

    #[test]
    fn check_order_ok() {
        let mut r = risk();
        assert!(r.check_order(&make_order(1.0), 10000.0).is_ok());
    }

    #[test]
    fn check_order_exceeds_max_position() {
        let mut r = risk();
        let result = r.check_order(&make_order(20.0), 10000.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exceeds max"));
    }

    #[test]
    fn check_order_drawdown_exceeded() {
        let mut r = risk();
        // peak_equity 10000, current equity 8000 → drawdown 20% > 10%
        assert!(r.check_order(&make_order(1.0), 10000.0).is_ok());
        let result = r.check_order(&make_order(1.0), 8000.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("drawdown"));
    }

    #[test]
    fn check_order_daily_loss_exceeded() {
        let mut r = risk();
        r.record_loss(1500.0);
        let result = r.check_order(&make_order(1.0), 10000.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("daily loss"));
    }

    #[test]
    fn check_order_rate_limit_exceeded() {
        let mut r = risk();
        r.window_start = Instant::now();
        for _ in 0..5 {
            assert!(r.check_order(&make_order(1.0), 10000.0).is_ok());
            r.record_trade();
        }
        let result = r.check_order(&make_order(1.0), 10000.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("rate limit"));
    }

    #[test]
    fn check_cooldown_rejects_orders() {
        let mut r = risk();
        r.in_cooldown = true;
        r.cooldown_end = Instant::now() + Duration::from_secs(3600);
        let result = r.check_order(&make_order(1.0), 10000.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cooldown"));
    }

    #[test]
    fn cooldown_expires() {
        let mut r = risk();
        r.in_cooldown = true;
        r.cooldown_end = Instant::now() - Duration::from_secs(1);
        assert!(r.check_order(&make_order(1.0), 10000.0).is_ok());
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
        r.check_order(&make_order(1.0), 5000.0).ok();
        assert_eq!(r.peak_equity, 5000.0);
        r.check_order(&make_order(1.0), 6000.0).ok();
        assert_eq!(r.peak_equity, 6000.0);
        r.check_order(&make_order(1.0), 4000.0).ok();
        assert_eq!(r.peak_equity, 6000.0);
    }

    #[test]
    fn zero_peak_equity_skips_drawdown() {
        let mut r = risk();
        assert!(r.check_order(&make_order(1.0), 0.0).is_ok());
    }

    #[test]
    fn rate_limit_window_resets_after_one_sec() {
        let mut r = risk();
        r.window_start = Instant::now() - Duration::from_secs(2);
        r.order_count = 10;
        assert!(r.check_order(&make_order(1.0), 10000.0).is_ok());
        assert_eq!(r.order_count, 0);
        r.record_trade();
        assert_eq!(r.order_count, 1);
    }
}
