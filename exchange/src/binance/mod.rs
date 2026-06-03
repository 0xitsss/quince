pub mod public;
pub mod ws;
pub mod types;

use crate::r#trait::{Exchange, ExchangeError, OrderStatus, Result, Stream};
use crossbeam_channel;
use quince_core::types::*;
use serde_json::{Map, Value};
use std::sync::OnceLock;

pub struct Binance {
    api_key: String,
    secret_key: String,
    testnet: bool,
    client: OnceLock<ws::WsClient>,
}

impl Binance {
    pub fn new(api_key: &str, secret_key: &str, testnet: bool) -> Self {
        Self {
            api_key: api_key.to_string(),
            secret_key: secret_key.to_string(),
            testnet,
            client: OnceLock::new(),
        }
    }

    fn req_tx(&self) -> Result<crossbeam_channel::Sender<ws::WsRequest>> {
        self.client
            .get()
            .map(|c| c.req_tx.clone())
            .ok_or(ExchangeError::Disconnected)
    }

    fn parse_account_info(result: Value) -> Result<AccountInfo> {
        let mut balances = Vec::with_capacity(
            result["assets"].as_array().map_or(0, |a| a.len()),
        );
        if let Some(assets) = result["assets"].as_array() {
            for a in assets {
                balances.push(Balance {
                    asset: a["asset"].as_str().unwrap_or("").to_string(),
                    wallet: a["walletBalance"]
                        .as_str()
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0.0),
                    cross_wallet: a["crossWalletBalance"]
                        .as_str()
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0.0),
                });
            }
        }

        let mut positions = Vec::with_capacity(
            result["positions"].as_array().map_or(0, |a| a.len()),
        );
        if let Some(poss) = result["positions"].as_array() {
            for p in poss {
                let size: f64 = p["positionAmt"]
                    .as_str()
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0.0);
                let side = if size > 0.0 {
                    PositionSide::Long
                } else if size < 0.0 {
                    PositionSide::Short
                } else {
                    PositionSide::None
                };
                positions.push(Position {
                    symbol: p["symbol"].as_str().unwrap_or("").to_string(),
                    side,
                    size: size.abs(),
                    entry_price: p["entryPrice"]
                        .as_str()
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0.0),
                    unrealized_pnl: p["unrealizedProfit"]
                        .as_str()
                        .unwrap_or("0")
                        .parse()
                        .unwrap_or(0.0),
                });
            }
        }

        Ok(AccountInfo { balances, positions })
    }
}

#[async_trait::async_trait]
impl Exchange for Binance {
    async fn subscribe(&self, symbols: &[String]) -> Result<Stream> {
        let ws = ws::BinanceWs::new(&self.api_key, &self.secret_key, self.testnet);
        let (client, rx) = ws.connect(symbols).await?;
        let _ = self.client.set(client);
        Ok(Stream { rx })
    }

    async fn place_order(&self, order: Order) -> Result<String> {
        let mut params = Map::new();
        params.insert(
            "symbol".into(),
            Value::String(order.symbol.to_uppercase()),
        );
        params.insert(
            "side".into(),
            Value::String(match order.side {
                Side::Buy => "BUY".into(),
                Side::Sell => "SELL".into(),
            }),
        );
        params.insert(
            "type".into(),
            Value::String(match order.order_type {
                OrderType::Market => "MARKET".into(),
                OrderType::Limit => "LIMIT".into(),
            }),
        );
        params.insert("quantity".into(), Value::String(format!("{}", order.qty)));
        if order.reduce_only {
            params.insert("reduceOnly".into(), Value::String("true".into()));
        }
        if let Some(price) = order.price {
            params.insert("price".into(), Value::String(format!("{}", price)));
        }

        let req_tx = self.req_tx()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        req_tx
            .try_send(ws::WsRequest {
                method: "order.place".into(),
                params,
                response_tx: tx,
            })
            .map_err(|_| ExchangeError::Disconnected)?;
        let result = rx.await.map_err(|_| ExchangeError::Disconnected)??;

        result["orderId"]
            .as_u64()
            .map(|id| id.to_string())
            .ok_or_else(|| ExchangeError::Order("missing orderId".into()))
    }

    async fn cancel_order(&self, symbol: &str, order_id: &str) -> Result<()> {
        let mut params = Map::new();
        params.insert(
            "symbol".into(),
            Value::String(symbol.to_uppercase()),
        );
        params.insert("orderId".into(), Value::String(order_id.into()));
        let req_tx = self.req_tx()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        req_tx
            .try_send(ws::WsRequest {
                method: "order.cancel".into(),
                params,
                response_tx: tx,
            })
            .map_err(|_| ExchangeError::Disconnected)?;
        rx.await.map_err(|_| ExchangeError::Disconnected)??;
        Ok(())
    }

    async fn order_status(&self, symbol: &str, order_id: &str) -> Result<OrderStatus> {
        let mut params = Map::new();
        params.insert(
            "symbol".into(),
            Value::String(symbol.to_uppercase()),
        );
        params.insert("orderId".into(), Value::String(order_id.into()));
        let req_tx = self.req_tx()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        req_tx
            .try_send(ws::WsRequest {
                method: "order.status".into(),
                params,
                response_tx: tx,
            })
            .map_err(|_| ExchangeError::Disconnected)?;
        let result = rx.await.map_err(|_| ExchangeError::Disconnected)??;

        Ok(OrderStatus {
            order_id: result["orderId"]
                .as_u64()
                .map(|v| v.to_string())
                .unwrap_or_default(),
            symbol: result["symbol"].as_str().unwrap_or("").to_string(),
            side: Side::from_taker(result["side"].as_str().unwrap_or("")),
            qty: result["origQty"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0.0),
            filled_qty: result["executedQty"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0.0),
            price: result["price"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0.0),
            avg_price: result["avgPrice"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0.0),
            status: result["status"].as_str().unwrap_or("").to_string(),
        })
    }

    async fn account_info(&self) -> Result<AccountInfo> {
        let req_tx = self.req_tx()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        req_tx
            .try_send(ws::WsRequest {
                method: "account.info".into(),
                params: Map::new(),
                response_tx: tx,
            })
            .map_err(|_| ExchangeError::Disconnected)?;
        let result = rx.await.map_err(|_| ExchangeError::Disconnected)??;
        Self::parse_account_info(result)
    }

    async fn current_price(&self, symbol: &str) -> Result<f64> {
        let mut params = Map::new();
        params.insert(
            "symbol".into(),
            Value::String(symbol.to_uppercase()),
        );
        let req_tx = self.req_tx()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        req_tx
            .try_send(ws::WsRequest {
                method: "ticker.price".into(),
                params,
                response_tx: tx,
            })
            .map_err(|_| ExchangeError::Disconnected)?;
        let result = rx.await.map_err(|_| ExchangeError::Disconnected)??;
        result["price"]
            .as_str()
            .unwrap_or("0")
            .parse()
            .map_err(|_| ExchangeError::Rest("parse error".into()))
    }
}
