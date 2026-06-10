// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Exchange trait definitions and shared types.
//! Defines [`Exchange`], [`ExchangeError`], [`StreamMsg`], [`OrderStatus`],
//! and the [`Stream`] subscription handle used by all exchange backends.

use quince_core::types::*;

pub type Result<T> = std::result::Result<T, ExchangeError>;

#[derive(Debug, thiserror::Error)]
pub enum ExchangeError {
    #[error("WebSocket error: {0}")]
    Ws(String),
    #[error("REST API error: {0}")]
    Rest(String),
    #[error("Authentication error: {0}")]
    Auth(String),
    #[error("Order failed: {0}")]
    Order(String),
    #[error("Timeout")]
    Timeout,
    #[error("Disconnected")]
    Disconnected,
}

pub struct Stream {
    pub rx: crossbeam_channel::Receiver<StreamMsg>,
}

#[derive(Debug)]
pub enum StreamMsg {
    Trade(Trade),
    Depth(Depth),
    MarkPrice {
        price: f64,
        time: chrono::DateTime<chrono::Utc>,
    },
    OpenInterest {
        qty: f64,
        time: chrono::DateTime<chrono::Utc>,
    },
    ForceOrder(Trade),
    AccountUpdate(AccountInfo),
    OrderUpdate(OrderFill),
}

#[async_trait::async_trait]
pub trait Exchange: Send + Sync {
    async fn subscribe(&self, symbols: &[String]) -> Result<Stream>;
    async fn place_order(&self, order: Order) -> Result<String>;
    async fn cancel_order(&self, symbol: &str, order_id: &str) -> Result<()>;
    async fn order_status(&self, symbol: &str, order_id: &str) -> Result<OrderStatus>;
    async fn account_info(&self) -> Result<AccountInfo>;
    async fn current_price(&self, symbol: &str) -> Result<f64>;
}

#[derive(Debug, Clone)]
pub struct OrderStatus {
    pub order_id: String,
    pub symbol: String,
    pub side: Side,
    pub qty: f64,
    pub filled_qty: f64,
    pub price: f64,
    pub avg_price: f64,
    pub status: String,
}
