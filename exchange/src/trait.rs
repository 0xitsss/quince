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

/// An order paired with the caller-generated idempotency key.
///
/// The key is created by the engine before submission and must remain stable
/// across transport failures.  Adapters that support native client IDs must
/// send it verbatim and expose lookup by it for reconciliation.
#[derive(Debug, Clone)]
pub struct OrderRequest {
    pub client_order_id: String,
    pub order: Order,
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
    /// The private stream had a gap or an integrity failure. Consumers must
    /// fetch authoritative account/order state before trusting it again.
    ReconcileRequired {
        source: &'static str,
        reason: String,
    },
}

#[async_trait::async_trait]
pub trait Exchange: Send + Sync {
    async fn subscribe(&self, symbols: &[String]) -> Result<Stream>;
    async fn place_order(&self, request: OrderRequest) -> Result<String>;
    async fn cancel_order(&self, symbol: &str, order_id: &str) -> Result<()>;
    async fn order_status(&self, symbol: &str, order_id: &str) -> Result<OrderStatus>;
    /// Looks up an order by the caller-generated client ID.
    ///
    /// Read-only and incomplete adapters keep the fail-closed default. Engine
    /// reconciliation uses this only after an ambiguous submission outcome.
    async fn order_status_by_client_id(
        &self,
        _symbol: &str,
        _client_order_id: &str,
    ) -> Result<OrderStatus> {
        Err(ExchangeError::Order(
            "exchange does not support client-order-id reconciliation".into(),
        ))
    }
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
