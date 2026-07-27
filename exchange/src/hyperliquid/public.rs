// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Read-only Hyperliquid market-data adapter.
//!
//! Subscribes to the official `trades` and `l2Book` WebSocket feeds. Trading
//! is deliberately rejected here: Hyperliquid requires EIP-712 action signing,
//! which must be implemented as a dedicated authenticated adapter.

use crate::r#trait::{
    Exchange, ExchangeError, OrderRequest, OrderStatus, Result, Stream, StreamMsg,
};
use futures_util::{SinkExt, StreamExt};
use quince_core::types::*;

#[derive(Debug, Clone, Copy)]
pub struct HyperliquidPublic {
    testnet: bool,
}

impl HyperliquidPublic {
    pub fn new(testnet: bool) -> Self {
        Self { testnet }
    }

    fn ws_url(&self) -> &'static str {
        if self.testnet {
            "wss://api.hyperliquid-testnet.xyz/ws"
        } else {
            "wss://api.hyperliquid.xyz/ws"
        }
    }

    fn http_url(&self) -> &'static str {
        if self.testnet {
            "https://api.hyperliquid-testnet.xyz/info"
        } else {
            "https://api.hyperliquid.xyz/info"
        }
    }
}

#[async_trait::async_trait]
impl Exchange for HyperliquidPublic {
    async fn subscribe(&self, symbols: &[String]) -> Result<Stream> {
        let (mut writer, mut reader) = tokio_tungstenite::connect_async(self.ws_url())
            .await
            .map_err(|e| ExchangeError::Ws(e.to_string()))?
            .0
            .split();

        for symbol in symbols {
            let coin = symbol.to_ascii_uppercase();
            for subscription in ["trades", "l2Book"] {
                let payload = serde_json::json!({
                    "method": "subscribe",
                    "subscription": { "type": subscription, "coin": coin },
                });
                writer
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        payload.to_string(),
                    ))
                    .await
                    .map_err(|e| ExchangeError::Ws(e.to_string()))?;
            }
        }

        let (tx, rx) = crossbeam_channel::bounded(1024);
        tokio::spawn(async move {
            while let Some(Ok(message)) = reader.next().await {
                if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
                    if let Some(event) = parse_ws_msg(&text) {
                        let _ = tx.try_send(event);
                    }
                }
            }
        });
        Ok(Stream { rx })
    }

    async fn place_order(&self, _request: OrderRequest) -> Result<String> {
        Err(ExchangeError::Order(
            "Hyperliquid public adapter is read-only; authenticated signing is required".into(),
        ))
    }

    async fn cancel_order(&self, _symbol: &str, _order_id: &str) -> Result<()> {
        Err(ExchangeError::Order(
            "Hyperliquid public adapter is read-only".into(),
        ))
    }

    async fn order_status(&self, _symbol: &str, _order_id: &str) -> Result<OrderStatus> {
        Err(ExchangeError::Order(
            "Hyperliquid public adapter is read-only".into(),
        ))
    }

    async fn account_info(&self) -> Result<AccountInfo> {
        Ok(AccountInfo {
            balances: Vec::new(),
            positions: Vec::new(),
        })
    }

    async fn current_price(&self, symbol: &str) -> Result<f64> {
        let client = reqwest::Client::new();
        let response: serde_json::Value = client
            .post(self.http_url())
            .json(&serde_json::json!({ "type": "allMids" }))
            .send()
            .await
            .map_err(|e| ExchangeError::Rest(e.to_string()))?
            .error_for_status()
            .map_err(|e| ExchangeError::Rest(e.to_string()))?
            .json()
            .await
            .map_err(|e| ExchangeError::Rest(e.to_string()))?;
        response[symbol.to_ascii_uppercase()]
            .as_str()
            .ok_or_else(|| ExchangeError::Rest(format!("missing mid price for {symbol}")))?
            .parse()
            .map_err(|_| ExchangeError::Rest(format!("invalid mid price for {symbol}")))
    }
}

fn parse_number(value: &serde_json::Value) -> Option<f64> {
    value.as_str()?.parse().ok()
}

fn parse_ws_msg(text: &str) -> Option<StreamMsg> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    match value.get("channel")?.as_str()? {
        "trades" => {
            let trade = value.get("data")?.as_array()?.first()?;
            Some(StreamMsg::Trade(Trade {
                price: parse_number(trade.get("px")?)?,
                qty: parse_number(trade.get("sz")?)?,
                time: chrono::DateTime::from_timestamp_millis(trade.get("time")?.as_i64()?)?,
                side: match trade.get("side")?.as_str()? {
                    "B" | "buy" => Side::Buy,
                    "A" | "sell" => Side::Sell,
                    _ => return None,
                },
                trade_id: trade.get("tid")?.as_u64()?,
            }))
        }
        "l2Book" => {
            let levels = value.get("data")?.get("levels")?.as_array()?;
            let parse_side = |index: usize| {
                levels
                    .get(index)?
                    .as_array()?
                    .iter()
                    .map(|level| {
                        Some(DepthLevel {
                            price: parse_number(level.get("px")?)?,
                            qty: parse_number(level.get("sz")?)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()
            };
            let bids = parse_side(0)?;
            let asks = parse_side(1)?;
            Some(StreamMsg::Depth(Depth { bids, asks }))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_trade_message() {
        let event = parse_ws_msg(r#"{"channel":"trades","data":[{"coin":"BTC","side":"B","px":"65000","sz":"0.1","time":1717200000000,"tid":7}]}"#).unwrap();
        assert!(
            matches!(event, StreamMsg::Trade(Trade { price, qty, side: Side::Buy, trade_id: 7, .. }) if price == 65000.0 && qty == 0.1)
        );
    }

    #[test]
    fn parses_l2_book_message() {
        let event = parse_ws_msg(r#"{"channel":"l2Book","data":{"levels":[[{"px":"100","sz":"2"}],[{"px":"101","sz":"3"}]]}}"#).unwrap();
        assert!(
            matches!(event, StreamMsg::Depth(Depth { bids, asks }) if bids[0].price == 100.0 && asks[0].qty == 3.0)
        );
    }

    #[test]
    fn uses_official_testnet_url() {
        assert_eq!(
            HyperliquidPublic::new(true).ws_url(),
            "wss://api.hyperliquid-testnet.xyz/ws"
        );
    }
}
