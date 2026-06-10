// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Core domain types shared across all Quince crates.
//! Defines [`Trade`], [`Side`], [`Depth`], [`Order`], [`Position`], [`Balance`],
//! and related types used throughout the trading pipeline.

use std::sync::Arc;

use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy)]
pub struct Trade {
    pub price: f64,
    pub qty: f64,
    pub time: DateTime<Utc>,
    pub side: Side,
    pub trade_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn from_taker(side: &str) -> Self {
        match side {
            "buy" | "BUY" | "takerBuy" => Side::Buy,
            _ => Side::Sell,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DepthLevel {
    pub price: f64,
    pub qty: f64,
}

#[derive(Debug, Clone)]
pub struct Depth {
    pub bids: Vec<DepthLevel>,
    pub asks: Vec<DepthLevel>,
}

#[derive(Debug, Clone)]
pub struct Order {
    pub symbol: Arc<str>,
    pub side: Side,
    pub qty: f64,
    pub price: Option<f64>,
    pub order_type: OrderType,
    pub reduce_only: bool,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderType {
    Market,
    Limit,
}

#[derive(Debug, Clone)]
pub struct OrderFill {
    pub order_id: String,
    pub side: Side,
    pub price: f64,
    pub qty: f64,
    pub fee: f64,
    pub fee_asset: String,
    pub time: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub balances: Vec<Balance>,
    pub positions: Vec<Position>,
}

#[derive(Debug, Clone)]
pub struct Balance {
    pub asset: String,
    pub wallet: f64,
    pub cross_wallet: f64,
}

#[derive(Debug, Clone)]
pub struct Position {
    pub symbol: String,
    pub side: PositionSide,
    pub size: f64,
    pub entry_price: f64,
    pub unrealized_pnl: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PositionSide {
    Long,
    Short,
    None,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_from_taker_buy() {
        assert_eq!(Side::from_taker("buy"), Side::Buy);
    }

    #[test]
    fn side_from_taker_buy_upper() {
        assert_eq!(Side::from_taker("BUY"), Side::Buy);
    }

    #[test]
    fn side_from_taker_buy_stream() {
        assert_eq!(Side::from_taker("takerBuy"), Side::Buy);
    }

    #[test]
    fn side_from_taker_sell_default() {
        assert_eq!(Side::from_taker("sell"), Side::Sell);
    }

    #[test]
    fn side_from_taker_unknown() {
        assert_eq!(Side::from_taker("unknown"), Side::Sell);
    }

    #[test]
    fn side_debug_buy() {
        assert_eq!(format!("{:?}", Side::Buy), "Buy");
    }

    #[test]
    fn side_partial_eq() {
        assert_eq!(Side::Buy, Side::Buy);
        assert_ne!(Side::Buy, Side::Sell);
    }

    #[test]
    fn order_type_partial_eq() {
        assert_eq!(OrderType::Market, OrderType::Market);
        assert_ne!(OrderType::Market, OrderType::Limit);
    }

    #[test]
    fn position_side_partial_eq() {
        assert_eq!(PositionSide::Long, PositionSide::Long);
        assert_eq!(PositionSide::Short, PositionSide::Short);
        assert_eq!(PositionSide::None, PositionSide::None);
    }

    #[test]
    fn trade_default_fields() {
        let t = Trade {
            price: 50000.0,
            qty: 0.1,
            time: chrono::Utc::now(),
            side: Side::Buy,
            trade_id: 1,
        };
        assert_eq!(t.price, 50000.0);
        assert_eq!(t.trade_id, 1);
    }

    #[test]
    fn depth_level_default() {
        let dl = DepthLevel {
            price: 100.0,
            qty: 1.5,
        };
        assert_eq!(dl.price, 100.0);
        assert_eq!(dl.qty, 1.5);
    }

    #[test]
    fn order_new_market() {
        let o = Order {
            symbol: "btcusdt".into(),
            side: Side::Buy,
            qty: 1.0,
            price: None,
            order_type: OrderType::Market,
            reduce_only: false,
            stop_loss: None,
            take_profit: None,
        };
        assert_eq!(o.symbol.as_ref(), "btcusdt");
        assert_eq!(o.order_type, OrderType::Market);
    }

    #[test]
    fn order_fill_has_fee() {
        let f = OrderFill {
            order_id: "test".into(),
            side: Side::Sell,
            price: 50000.0,
            qty: 0.5,
            fee: 25.0,
            fee_asset: "USDT".into(),
            time: chrono::Utc::now(),
        };
        assert_eq!(f.fee, 25.0);
        assert_eq!(f.fee_asset, "USDT");
    }
}
