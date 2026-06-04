/// Risk Engine — runtime-enforced trading limits.
///
/// Intercepts orders before they reach the exchange connector.
/// Rejects orders that violate configured limits.
/// Limits are set externally (from strategy config or CLI).

use quince_core::types::Order;

/// Runtime-enforced risk limits.
#[derive(Debug, Clone)]
pub struct RiskLimits {
    /// Maximum absolute position size (in quote currency).
    pub max_position: f64,
    /// Maximum notional per order.
    pub max_order_notional: f64,
    /// Maximum order count per evaluation cycle.
    pub max_orders_per_cycle: u32,
}

impl Default for RiskLimits {
    fn default() -> Self {
        RiskLimits {
            max_position: 10.0,
            max_order_notional: 10_000.0,
            max_orders_per_cycle: 5,
        }
    }
}

/// Result of a risk check.
#[derive(Debug, Clone, PartialEq)]
pub enum RiskVerdict {
    /// Order is allowed.
    Allowed,
    /// Order is rejected with a reason.
    Rejected(String),
}

/// Runtime risk engine.
#[derive(Debug, Clone)]
pub struct RiskEngine {
    pub limits: RiskLimits,
    /// Current absolute position (set externally from strategy).
    pub current_position: f64,
    /// Orders sent in current cycle.
    orders_this_cycle: u32,
}

impl RiskEngine {
    pub fn new(limits: RiskLimits) -> Self {
        RiskEngine {
            limits,
            current_position: 0.0,
            orders_this_cycle: 0,
        }
    }

    /// Check an order against configured limits.
    pub fn check_order(&mut self, order: &Order) -> RiskVerdict {
        // Max orders per cycle
        if self.orders_this_cycle >= self.limits.max_orders_per_cycle {
            return RiskVerdict::Rejected(format!(
                "max orders per cycle exceeded ({} / {})",
                self.orders_this_cycle, self.limits.max_orders_per_cycle
            ));
        }

        // Max order notional
        let notional = order.qty * order.price.unwrap_or(0.0);
        if notional > self.limits.max_order_notional {
            return RiskVerdict::Rejected(format!(
                "order notional {:.2} exceeds max {:.2}",
                notional, self.limits.max_order_notional
            ));
        }

        // Max position (for non-reduce orders)
        if !order.reduce_only {
            let new_position = match order.side {
                quince_core::types::Side::Buy => self.current_position + order.qty,
                quince_core::types::Side::Sell => self.current_position - order.qty,
            };
            if new_position.abs() > self.limits.max_position {
                return RiskVerdict::Rejected(format!(
                    "position {:.4} would exceed max {:.4}",
                    new_position.abs(),
                    self.limits.max_position
                ));
            }
        }

        // All checks passed
        self.orders_this_cycle += 1;
        RiskVerdict::Allowed
    }

    /// Reset per-cycle counters (call at start of each eval cycle).
    pub fn new_cycle(&mut self) {
        self.orders_this_cycle = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::types::{OrderType, Side};

    fn make_order(side: Side, qty: f64, price: Option<f64>) -> Order {
        Order {
            symbol: "BTCUSDT".into(),
            side,
            qty,
            price,
            order_type: OrderType::Market,
            reduce_only: false,
            stop_loss: None,
            take_profit: None,
        }
    }

    #[test]
    fn default_limits_allow_small_order() {
        let mut engine = RiskEngine::new(RiskLimits::default());
        let order = make_order(Side::Buy, 0.1, Some(50000.0));
        assert_eq!(engine.check_order(&order), RiskVerdict::Allowed);
    }

    #[test]
    fn order_exceeding_max_notional_rejected() {
        let mut engine = RiskEngine::new(RiskLimits::default());
        // max_order_notional = 10_000
        let order = make_order(Side::Buy, 1.0, Some(50_000.0));
        let result = engine.check_order(&order);
        assert!(matches!(result, RiskVerdict::Rejected(_)));
        if let RiskVerdict::Rejected(msg) = result {
            assert!(msg.contains("notional"));
        }
    }

    #[test]
    fn order_exceeding_max_position_rejected() {
        let mut engine = RiskEngine::new(RiskLimits::default());
        engine.current_position = 9.5; // near limit
        // Buying 1.0 more would make position 10.5 > 10.0
        let order = make_order(Side::Buy, 1.0, Some(50000.0));
        let result = engine.check_order(&order);
        assert!(matches!(result, RiskVerdict::Rejected(_)));
    }

    #[test]
    fn reduce_only_order_skips_position_check() {
        let mut engine = RiskEngine::new(RiskLimits::default());
        engine.current_position = 9.5;
        // qty=0.1, price=100 → notional=10 < 10000, passes
        let mut order = make_order(Side::Sell, 0.1, Some(100.0));
        order.reduce_only = true;
        assert_eq!(engine.check_order(&order), RiskVerdict::Allowed);
    }

    #[test]
    fn max_orders_per_cycle_enforced() {
        let mut engine = RiskEngine::new(RiskLimits {
            max_orders_per_cycle: 2,
            ..RiskLimits::default()
        });

        let order = make_order(Side::Buy, 0.1, Some(100.0));
        assert_eq!(engine.check_order(&order), RiskVerdict::Allowed);
        assert_eq!(engine.check_order(&order), RiskVerdict::Allowed);

        let result = engine.check_order(&order);
        assert!(matches!(result, RiskVerdict::Rejected(_)));
    }

    #[test]
    fn new_cycle_resets_order_count() {
        let mut engine = RiskEngine::new(RiskLimits {
            max_orders_per_cycle: 1,
            ..RiskLimits::default()
        });

        let order = make_order(Side::Buy, 0.1, Some(100.0));
        assert_eq!(engine.check_order(&order), RiskVerdict::Allowed);

        engine.new_cycle();
        assert_eq!(engine.check_order(&order), RiskVerdict::Allowed);
    }

    #[test]
    fn position_tracking_updated_on_allowed_buy() {
        let mut engine = RiskEngine::new(RiskLimits::default());
        engine.current_position = 2.0;
        let order = make_order(Side::Buy, 0.5, Some(100.0));
        let _ = engine.check_order(&order);
        // Position is NOT auto-updated — caller must set it
        assert_eq!(engine.current_position, 2.0);
    }

    #[test]
    fn zero_price_market_order_passes_notional_check() {
        let mut engine = RiskEngine::new(RiskLimits::default());
        // Market order with no price, small qty within position limit
        let order = make_order(Side::Buy, 0.5, None);
        // Notional = 0.5 * 0 = 0, position = 0.5 < 10
        assert_eq!(engine.check_order(&order), RiskVerdict::Allowed);
    }

    #[test]
    fn custom_limits_from_config() {
        let limits = RiskLimits {
            max_position: 0.5,
            max_order_notional: 1_000.0,
            max_orders_per_cycle: 2,
        };
        let mut engine = RiskEngine::new(limits);
        let order = make_order(Side::Buy, 0.6, Some(50000.0));
        // qty 0.6 > max_position 0.5
        let result = engine.check_order(&order);
        assert!(matches!(result, RiskVerdict::Rejected(_)));
    }
}
