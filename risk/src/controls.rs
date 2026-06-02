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

    pub fn check_order(
        &mut self,
        order: &Order,
        current_pnl: f64,
        current_equity: f64,
    ) -> Result<(), String> {
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

        if current_pnl < -self.max_daily_loss {
            return Err(format!(
                "daily loss {:.2} exceeds limit {:.2}",
                -current_pnl, self.max_daily_loss
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
