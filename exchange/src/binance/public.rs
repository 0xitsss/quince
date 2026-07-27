// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Read-only Binance exchange for public market data.
//! [`BinancePublic`] implements the [`Exchange`] trait without authentication,
//! supporting trade/depth subscriptions via combined WebSocket streams.

use crate::r#trait::{Exchange, ExchangeError, OrderRequest, OrderStatus, Result, Stream};
use futures_util::StreamExt;
use quince_core::types::*;

#[derive(Default)]
pub struct BinancePublic;

impl BinancePublic {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl Exchange for BinancePublic {
    async fn subscribe(&self, symbols: &[String]) -> Result<Stream> {
        let streams: Vec<String> = symbols
            .iter()
            .flat_map(|s| {
                let s = s.to_lowercase();
                vec![format!("{}@aggTrade", s), format!("{}@depth20@100ms", s)]
            })
            .collect();

        let url = format!(
            "wss://stream.binance.com:9443/stream?streams={}",
            streams.join("/")
        );

        tracing::info!("connecting to Binance public WS: {url}");

        let (ws_stream, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| ExchangeError::Ws(e.to_string()))?;

        tracing::info!("connected — subscribed streams: {:?}", streams);

        let (_, mut reader) = ws_stream.split();
        let (tx, rx) = crossbeam_channel::bounded(1024);

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(Ok(msg)) = reader.next() => {
                        if let tokio_tungstenite::tungstenite::Message::Text(text) = msg {
                            if let Some(stream_msg) = super::types::parse_ws_msg(text) {
                                let _ = tx.try_send(stream_msg);
                            }
                        }
                    }
                    else => break,
                }
            }
        });

        Ok(Stream { rx })
    }

    async fn place_order(&self, _request: OrderRequest) -> Result<String> {
        Err(ExchangeError::Order(
            "Binance public adapter is read-only; configure API credentials for trading".into(),
        ))
    }

    async fn cancel_order(&self, _symbol: &str, _order_id: &str) -> Result<()> {
        Err(ExchangeError::Order(
            "Binance public adapter is read-only".into(),
        ))
    }

    async fn order_status(&self, _symbol: &str, _order_id: &str) -> Result<OrderStatus> {
        Err(ExchangeError::Order(
            "Binance public adapter is read-only".into(),
        ))
    }

    async fn account_info(&self) -> Result<AccountInfo> {
        Ok(AccountInfo {
            balances: vec![
                Balance {
                    asset: "USDT".into(),
                    wallet: 10000.0,
                    cross_wallet: 10000.0,
                },
                Balance {
                    asset: "BTC".into(),
                    wallet: 0.1,
                    cross_wallet: 0.1,
                },
            ],
            positions: vec![],
        })
    }

    async fn current_price(&self, symbol: &str) -> Result<f64> {
        let url = format!(
            "https://api.binance.com/api/v3/ticker/price?symbol={}",
            symbol.to_uppercase()
        );
        let resp = reqwest::get(&url)
            .await
            .map_err(|e| ExchangeError::Rest(e.to_string()))?;
        let val: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ExchangeError::Rest(e.to_string()))?;
        val["price"]
            .as_str()
            .unwrap_or("0")
            .parse()
            .map_err(|_| ExchangeError::Rest("parse error".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#trait::Exchange;

    #[test]
    fn new_creates_exchange() {
        let ex = BinancePublic::new();
        assert_eq!(std::mem::size_of_val(&ex), 0);
    }

    #[tokio::test]
    async fn place_order_is_rejected_by_read_only_adapter() {
        let ex = BinancePublic::new();
        let order = Order {
            symbol: "BTCUSDT".into(),
            side: Side::Buy,
            qty: 1.0,
            price: None,
            order_type: OrderType::Market,
            reduce_only: false,
            stop_loss: None,
            take_profit: None,
        };
        assert!(ex
            .place_order(OrderRequest {
                client_order_id: "test-order".into(),
                order,
            })
            .await
            .is_err());
    }

    #[tokio::test]
    async fn cancel_order_is_rejected_by_read_only_adapter() {
        let ex = BinancePublic::new();
        assert!(ex.cancel_order("BTCUSDT", "12345").await.is_err());
    }

    #[tokio::test]
    async fn order_status_is_rejected_by_read_only_adapter() {
        let ex = BinancePublic::new();
        assert!(ex.order_status("ETHUSDT", "ignored").await.is_err());
    }

    #[tokio::test]
    async fn account_info_has_usdt_and_btc() {
        let ex = BinancePublic::new();
        let info = ex.account_info().await.unwrap();
        let usdt = info
            .balances
            .iter()
            .find(|b| b.asset == "USDT")
            .expect("USDT balance");
        assert_eq!(usdt.wallet, 10000.0);
        assert_eq!(usdt.cross_wallet, 10000.0);

        let btc = info
            .balances
            .iter()
            .find(|b| b.asset == "BTC")
            .expect("BTC balance");
        assert!((btc.wallet - 0.1).abs() < 1e-10);
        assert!(info.positions.is_empty());
    }

    #[tokio::test]
    async fn account_info_balances_count() {
        let ex = BinancePublic::new();
        let info = ex.account_info().await.unwrap();
        assert_eq!(info.balances.len(), 2);
    }

    #[tokio::test]
    #[ignore = "requires network access to Binance REST API"]
    async fn current_price_btc() {
        let ex = BinancePublic::new();
        let price = ex.current_price("BTCUSDT").await.unwrap();
        assert!(price > 0.0, "BTC price should be positive, got: {price}");
    }

    #[tokio::test]
    #[ignore = "requires network access to Binance WebSocket"]
    async fn subscribe_receives_trade() {
        let ex = BinancePublic::new();
        let stream = ex.subscribe(&["btcusdt".into()]).await.unwrap();
        let msg = tokio::task::spawn_blocking(move || stream.rx.recv()).await;
        assert!(msg.is_ok(), "should receive a stream message within 30s");
    }
}
