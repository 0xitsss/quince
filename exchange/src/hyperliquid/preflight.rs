// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Fail-closed market-context checks for authenticated execution.
//!
//! This module is deliberately transport-free: a caller must bind an order to
//! a specific, fresh, finite market observation before it is signed.  A wall
//! clock timestamp alone is not evidence that a quote is usable for an order.

use crate::r#trait::{ExchangeError, Result};
use chrono::{DateTime, Duration, Utc};
use quince_core::types::Order;

/// Immutable quote evidence captured at the decision boundary.
#[derive(Debug, Clone)]
pub struct MarketSnapshot {
    /// Hyperliquid coin name for which the quote was observed.
    pub symbol: String,
    pub observed_at: DateTime<Utc>,
    /// A finite positive reference price, normally a mid or last-trade price.
    pub reference_price: f64,
}

/// Explicit bounds for accepting a market observation for execution.
#[derive(Debug, Clone, Copy)]
pub struct MarketContextPolicy {
    pub max_age: Duration,
    /// Maximum absolute limit-price deviation from the reference price.
    pub max_limit_deviation_bps: u32,
}

impl MarketContextPolicy {
    /// Validates a snapshot for this exact order. Every malformed value blocks
    /// signing; this function never substitutes a last-known quote.
    pub fn check(
        &self,
        order: &Order,
        snapshot: &MarketSnapshot,
        now: DateTime<Utc>,
    ) -> Result<()> {
        if self.max_age <= Duration::zero() {
            return Err(ExchangeError::Order("invalid market-data TTL".into()));
        }
        if self.max_limit_deviation_bps == 0 || self.max_limit_deviation_bps > 10_000 {
            return Err(ExchangeError::Order(
                "invalid Hyperliquid limit-price deviation policy".into(),
            ));
        }
        if !snapshot.reference_price.is_finite() || snapshot.reference_price <= 0.0 {
            return Err(ExchangeError::Order(
                "invalid Hyperliquid market reference price".into(),
            ));
        }
        if !snapshot.symbol.eq_ignore_ascii_case(order.symbol.trim()) {
            return Err(ExchangeError::Order(
                "Hyperliquid market snapshot symbol does not match order".into(),
            ));
        }

        let age = now.signed_duration_since(snapshot.observed_at);
        if age < Duration::zero() || age > self.max_age {
            return Err(ExchangeError::Order(
                "Hyperliquid execution blocked: market data is stale or clock-skewed".into(),
            ));
        }

        if let Some(limit_price) = order.price {
            if !limit_price.is_finite() || limit_price <= 0.0 {
                return Err(ExchangeError::Order(
                    "invalid Hyperliquid limit price".into(),
                ));
            }
            let deviation_bps = (limit_price - snapshot.reference_price).abs()
                / snapshot.reference_price
                * 10_000.0;
            if !deviation_bps.is_finite() || deviation_bps > f64::from(self.max_limit_deviation_bps)
            {
                return Err(ExchangeError::Order(
                    "Hyperliquid limit price exceeds execution deviation policy".into(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::types::{Order, OrderType, Side};

    fn order() -> Order {
        Order {
            symbol: "BTC".into(),
            side: Side::Buy,
            qty: 0.1,
            price: Some(100.0),
            order_type: OrderType::Limit,
            reduce_only: false,
            stop_loss: None,
            take_profit: None,
        }
    }

    fn policy() -> MarketContextPolicy {
        MarketContextPolicy {
            max_age: Duration::seconds(1),
            max_limit_deviation_bps: 100,
        }
    }

    #[test]
    fn accepts_fresh_matching_quote_within_explicit_deviation_bound() {
        let now = Utc::now();
        policy()
            .check(
                &order(),
                &MarketSnapshot {
                    symbol: "btc".into(),
                    observed_at: now,
                    reference_price: 100.5,
                },
                now,
            )
            .unwrap();
    }

    #[test]
    fn rejects_wrong_symbol_non_finite_quote_and_excessive_limit_deviation() {
        let now = Utc::now();
        let snapshot = MarketSnapshot {
            symbol: "ETH".into(),
            observed_at: now,
            reference_price: 100.0,
        };
        assert!(policy().check(&order(), &snapshot, now).is_err());

        let snapshot = MarketSnapshot {
            symbol: "BTC".into(),
            observed_at: now,
            reference_price: f64::NAN,
        };
        assert!(policy().check(&order(), &snapshot, now).is_err());

        let mut far_order = order();
        far_order.price = Some(102.0);
        let snapshot = MarketSnapshot {
            symbol: "BTC".into(),
            observed_at: now,
            reference_price: 100.0,
        };
        assert!(policy().check(&far_order, &snapshot, now).is_err());
    }

    #[test]
    fn rejects_stale_future_and_unbounded_policies() {
        let now = Utc::now();
        let snapshot = MarketSnapshot {
            symbol: "BTC".into(),
            observed_at: now - Duration::seconds(2),
            reference_price: 100.0,
        };
        assert!(policy().check(&order(), &snapshot, now).is_err());

        let future = MarketSnapshot {
            observed_at: now + Duration::milliseconds(1),
            ..snapshot
        };
        assert!(policy().check(&order(), &future, now).is_err());

        let invalid = MarketContextPolicy {
            max_age: Duration::seconds(1),
            max_limit_deviation_bps: 10_001,
        };
        assert!(invalid.check(&order(), &future, now).is_err());
    }
}
