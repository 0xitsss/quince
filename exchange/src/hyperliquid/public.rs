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

/// Keep one slot available for an integrity signal when market data bursts.
/// A full queue is not a harmless telemetry condition: consumers must discard
/// their assumptions and refresh authoritative state before trading again.
const STREAM_CHANNEL_CAPACITY: usize = 1024;

fn stream_integrity_event(reason: impl Into<String>) -> StreamMsg {
    StreamMsg::ReconcileRequired {
        source: "hyperliquid-public",
        reason: reason.into(),
    }
}

/// Returns whether the market-data event reached the engine.  The final slot
/// is deliberately reserved for `ReconcileRequired`, preventing an overflow
/// from being silently indistinguishable from a healthy stream.
fn relay_market_data(tx: &crossbeam_channel::Sender<StreamMsg>, event: StreamMsg) -> bool {
    if tx.len() >= STREAM_CHANNEL_CAPACITY - 1 {
        let _ = tx.try_send(stream_integrity_event("market-data queue overflow"));
        return false;
    }

    match tx.try_send(event) {
        Ok(()) => true,
        Err(crossbeam_channel::TrySendError::Full(_)) => {
            let _ = tx.try_send(stream_integrity_event("market-data queue overflow"));
            false
        }
        Err(crossbeam_channel::TrySendError::Disconnected(_)) => false,
    }
}

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

    /// Fetches the authoritative perp account snapshot for an address.
    ///
    /// Hyperliquid's `clearinghouseState` endpoint is read-only: it does not
    /// require a signature and is therefore safe to use for account/risk
    /// reconciliation before authenticated order submission exists.
    pub(crate) async fn account_info_for(&self, address: &str) -> Result<AccountInfo> {
        let client = reqwest::Client::new();
        let response: serde_json::Value = client
            .post(self.http_url())
            .json(&serde_json::json!({
                "type": "clearinghouseState",
                "user": address,
            }))
            .send()
            .await
            .map_err(|e| ExchangeError::Rest(format!("fetch Hyperliquid account state: {e}")))?
            .error_for_status()
            .map_err(|e| ExchangeError::Rest(format!("Hyperliquid account-state status: {e}")))?
            .json()
            .await
            .map_err(|e| ExchangeError::Rest(format!("parse Hyperliquid account state: {e}")))?;
        parse_account_info(&response)
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

        let (tx, rx) = crossbeam_channel::bounded(STREAM_CHANNEL_CAPACITY);
        tokio::spawn(async move {
            let terminal_reason = loop {
                match reader.next().await {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                        if let Some(event) = parse_ws_msg(&text) {
                            relay_market_data(&tx, event);
                        }
                    }
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Close(frame))) => {
                        break format!("websocket closed: {frame:?}");
                    }
                    Some(Ok(_)) => {}
                    Some(Err(error)) => break format!("websocket receive error: {error}"),
                    None => break "websocket closed without a close frame".into(),
                }
            };
            if tx
                .try_send(stream_integrity_event(terminal_reason))
                .is_err()
            {
                tracing::error!("Hyperliquid market-data stream ended while its queue was full");
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
    let number = value.as_str()?.parse::<f64>().ok()?;
    number.is_finite().then_some(number)
}

fn required_number(value: &serde_json::Value, field: &str) -> Result<f64> {
    parse_number(value).ok_or_else(|| ExchangeError::Rest(format!("invalid Hyperliquid {field}")))
}

fn parse_positive_number(value: &serde_json::Value) -> Option<f64> {
    let number = parse_number(value)?;
    (number > 0.0).then_some(number)
}

fn parse_account_info(value: &serde_json::Value) -> Result<AccountInfo> {
    let account_value = required_number(
        value
            .get("crossMarginSummary")
            .and_then(|summary| summary.get("accountValue"))
            .ok_or_else(|| {
                ExchangeError::Rest("missing Hyperliquid cross margin summary".into())
            })?,
        "cross margin account value",
    )?;
    if account_value < 0.0 {
        return Err(ExchangeError::Rest(
            "negative Hyperliquid cross margin account value".into(),
        ));
    }
    let positions = value
        .get("assetPositions")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| ExchangeError::Rest("missing Hyperliquid asset positions".into()))?
        .iter()
        .map(|entry| {
            let position = entry
                .get("position")
                .ok_or_else(|| ExchangeError::Rest("missing Hyperliquid position".into()))?;
            let symbol = position
                .get("coin")
                .and_then(serde_json::Value::as_str)
                .filter(|symbol| !symbol.is_empty())
                .ok_or_else(|| ExchangeError::Rest("invalid Hyperliquid position coin".into()))?;
            let signed_size = required_number(
                position.get("szi").ok_or_else(|| {
                    ExchangeError::Rest("missing Hyperliquid position size".into())
                })?,
                "position size",
            )?;
            let entry_price = match position.get("entryPx") {
                Some(serde_json::Value::Null) | None => 0.0,
                Some(value) => required_number(value, "position entry price")?,
            };
            let unrealized_pnl = required_number(
                position.get("unrealizedPnl").ok_or_else(|| {
                    ExchangeError::Rest("missing Hyperliquid position unrealized PnL".into())
                })?,
                "position unrealized PnL",
            )?;
            Ok(Position {
                symbol: symbol.to_owned(),
                side: if signed_size > 0.0 {
                    PositionSide::Long
                } else if signed_size < 0.0 {
                    PositionSide::Short
                } else {
                    PositionSide::None
                },
                size: signed_size.abs(),
                entry_price,
                unrealized_pnl,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(AccountInfo {
        balances: vec![Balance {
            // Perp collateral is denominated in USDC. `accountValue` includes
            // settled collateral and unrealized PnL; it is the appropriate
            // conservative balance for the engine's cross-margin risk view.
            asset: "USDC".into(),
            wallet: account_value,
            cross_wallet: account_value,
        }],
        positions,
    })
}

fn parse_ws_msg(text: &str) -> Option<StreamMsg> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    match value.get("channel")?.as_str()? {
        "trades" => {
            let trade = value.get("data")?.as_array()?.first()?;
            Some(StreamMsg::Trade(Trade {
                price: parse_positive_number(trade.get("px")?)?,
                qty: parse_positive_number(trade.get("sz")?)?,
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
            let parse_side = |index: usize, descending: bool| {
                levels
                    .get(index)?
                    .as_array()?
                    .iter()
                    .map(|level| {
                        Some(DepthLevel {
                            price: parse_positive_number(level.get("px")?)?,
                            qty: parse_positive_number(level.get("sz")?)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()
                    .filter(|side| {
                        !side.is_empty()
                            && !side.windows(2).any(|pair| {
                                if descending {
                                    pair[0].price <= pair[1].price
                                } else {
                                    pair[0].price >= pair[1].price
                                }
                            })
                    })
            };
            let bids = parse_side(0, true)?;
            let asks = parse_side(1, false)?;
            if bids.last()?.price >= asks.first()?.price {
                return None;
            }
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
    fn contract_fixture_public_trade_preserves_side_and_id() {
        let event = parse_ws_msg(include_str!("../../tests/fixtures/hyperliquid_trades.json"))
            .expect("Hyperliquid trade fixture must remain parseable");
        assert!(matches!(
            event,
            StreamMsg::Trade(Trade {
                side: Side::Buy,
                trade_id: 7,
                ..
            })
        ));
    }

    #[test]
    fn contract_fixture_l2_book_preserves_bid_ask_ordering() {
        let event = parse_ws_msg(include_str!(
            "../../tests/fixtures/hyperliquid_l2_book.json"
        ))
        .expect("Hyperliquid l2Book fixture must remain parseable");
        assert!(matches!(
            event,
            StreamMsg::Depth(Depth { bids, asks })
                if bids[0].price == 65000.0 && asks[0].price == 65001.0
        ));
    }

    #[test]
    fn contract_fixture_trade_batch_preserves_first_exchange_event() {
        let event = parse_ws_msg(include_str!(
            "../../tests/fixtures/hyperliquid_trade_batch.json"
        ))
        .expect("Hyperliquid batched trade fixture must remain parseable");
        assert!(matches!(
            event,
            StreamMsg::Trade(Trade {
                side: Side::Sell,
                price,
                qty,
                trade_id: 71_592_004,
                ..
            }) if price == 67_124.1 && qty == 0.019
        ));
    }

    #[test]
    fn contract_boundary_rejects_non_positive_or_non_finite_market_data() {
        for payload in [
            r#"{"channel":"trades","data":[{"side":"B","px":"NaN","sz":"1","time":1717200000000,"tid":7}]}"#,
            r#"{"channel":"trades","data":[{"side":"A","px":"100","sz":"0","time":1717200000000,"tid":7}]}"#,
            r#"{"channel":"l2Book","data":{"levels":[[{"px":"100","sz":"1"}],[{"px":"0","sz":"1"}]]}}"#,
        ] {
            assert!(parse_ws_msg(payload).is_none(), "{payload}");
        }
    }

    #[test]
    fn contract_matrix_hyperliquid_market_data_boundary() {
        let invalid = [
            "0", "-1", "NaN", "inf", "-inf", "abc", "", "  ", "1e309", "-0", "+0", "00",
        ];
        let mut cases = 0_u16;

        for value in invalid {
            for field in ["px", "sz"] {
                let mut trade = serde_json::json!({
                    "coin": "BTC", "side": "B", "px": "100", "sz": "1", "time": 1, "tid": 1
                });
                trade[field] = serde_json::Value::String(value.into());
                let payload = serde_json::json!({ "channel": "trades", "data": [trade] });
                assert!(
                    parse_ws_msg(&payload.to_string()).is_none(),
                    "trade {field}={value:?}"
                );
                cases += 1;
            }

            for side in [0, 1] {
                for field in ["px", "sz"] {
                    let mut level = serde_json::json!({ "px": "100", "sz": "1" });
                    level[field] = serde_json::Value::String(value.into());
                    let levels = if side == 0 {
                        serde_json::json!([[level], [{ "px": "101", "sz": "1" }]])
                    } else {
                        serde_json::json!([[{ "px": "100", "sz": "1" }], [level]])
                    };
                    let payload = serde_json::json!({
                        "channel": "l2Book", "data": { "levels": levels }
                    });
                    assert!(
                        parse_ws_msg(&payload.to_string()).is_none(),
                        "book side={side} {field}={value:?}"
                    );
                    cases += 1;
                }
            }
        }

        for side in ["", "BUY", "SELL", "X", "b", "a", "0", " "] {
            let payload = serde_json::json!({
                "channel": "trades",
                "data": [{ "side": side, "px": "100", "sz": "1", "time": 1, "tid": 1 }]
            });
            assert!(
                parse_ws_msg(&payload.to_string()).is_none(),
                "side={side:?}"
            );
            cases += 1;
        }

        for payload in [
            r#"{}"#,
            r#"{"channel":"trades"}"#,
            r#"{"channel":"trades","data":[]}"#,
            r#"{"channel":"trades","data":[{}]}"#,
            r#"{"channel":"trades","data":[{"side":"B","px":100,"sz":"1","time":1,"tid":1}]}"#,
            r#"{"channel":"trades","data":[{"side":"B","px":"100","sz":"1","time":"1","tid":1}]}"#,
            r#"{"channel":"trades","data":[{"side":"B","px":"100","sz":"1","time":1,"tid":"1"}]}"#,
            r#"{"channel":"l2Book","data":{"levels":[]}}"#,
            r#"{"channel":"l2Book","data":{"levels":[[],[{"px":"101","sz":"1"}]]}}"#,
            r#"{"channel":"l2Book","data":{"levels":[[{"px":"100","sz":"1"}],[]]}}"#,
            r#"{"channel":"l2Book","data":{"levels":[[{"px":"100","sz":"1"}],[{"px":"99","sz":"1"}]]}}"#,
            r#"{"channel":"l2Book","data":{"levels":[[{"px":"99","sz":"1"},{"px":"100","sz":"1"}],[{"px":"101","sz":"1"}]]}}"#,
            r#"{"channel":"l2Book","data":{"levels":[[{"px":"100","sz":"1"}],[{"px":"102","sz":"1"},{"px":"101","sz":"1"}]]}}"#,
            r#"not json"#,
        ] {
            assert!(parse_ws_msg(payload).is_none(), "{payload}");
            cases += 1;
        }

        for price in ["1", "100", "100.125", "1e-5"] {
            for qty in ["1", "2.5", "1e-8"] {
                for (wire_side, side) in [("B", Side::Buy), ("A", Side::Sell)] {
                    let payload = serde_json::json!({
                        "channel": "trades",
                        "data": [{ "side": wire_side, "px": price, "sz": qty, "time": 1, "tid": 1 }]
                    });
                    assert!(
                        matches!(parse_ws_msg(&payload.to_string()), Some(StreamMsg::Trade(Trade { side: actual, .. })) if actual == side)
                    );
                    cases += 1;
                }
            }
        }

        assert_eq!(cases, 118, "contract matrix coverage changed: {cases}");
    }

    #[test]
    fn uses_official_testnet_url() {
        assert_eq!(
            HyperliquidPublic::new(true).ws_url(),
            "wss://api.hyperliquid-testnet.xyz/ws"
        );
    }

    #[test]
    fn parses_cross_margin_account_snapshot_strictly() {
        let account = parse_account_info(&serde_json::json!({
            "crossMarginSummary": { "accountValue": "123.45" },
            "assetPositions": [
                { "position": { "coin": "BTC", "szi": "0.2", "entryPx": "60000", "unrealizedPnl": "4.5" } },
                { "position": { "coin": "ETH", "szi": "-3", "entryPx": "3000", "unrealizedPnl": "-2" } }
            ]
        }))
        .unwrap();
        assert_eq!(account.balances[0].asset, "USDC");
        assert_eq!(account.balances[0].wallet, 123.45);
        assert!(matches!(account.positions[0].side, PositionSide::Long));
        assert!(matches!(account.positions[1].side, PositionSide::Short));
        assert_eq!(account.positions[1].size, 3.0);
    }

    #[test]
    fn rejects_malformed_account_snapshot() {
        for snapshot in [
            serde_json::json!({}),
            serde_json::json!({ "crossMarginSummary": { "accountValue": "NaN" }, "assetPositions": [] }),
            serde_json::json!({ "crossMarginSummary": { "accountValue": "1" }, "assetPositions": [{ "position": { "coin": "BTC", "szi": "oops", "unrealizedPnl": "0" } }] }),
        ] {
            assert!(parse_account_info(&snapshot).is_err());
        }
    }

    #[test]
    fn websocket_close_is_reported_as_an_integrity_boundary() {
        let event = stream_integrity_event("websocket closed");
        assert!(
            matches!(event, StreamMsg::ReconcileRequired { source: "hyperliquid-public", reason } if reason == "websocket closed")
        );
    }

    #[test]
    fn queue_overflow_reserves_room_for_a_reconciliation_signal() {
        let (tx, rx) = crossbeam_channel::bounded(STREAM_CHANNEL_CAPACITY);
        for _ in 0..STREAM_CHANNEL_CAPACITY - 1 {
            tx.try_send(StreamMsg::OpenInterest {
                qty: 1.0,
                time: chrono::Utc::now(),
            })
            .unwrap();
        }

        assert!(!relay_market_data(
            &tx,
            StreamMsg::OpenInterest {
                qty: 2.0,
                time: chrono::Utc::now(),
            },
        ));
        assert!(matches!(
            rx.try_iter().last(),
            Some(StreamMsg::ReconcileRequired { source: "hyperliquid-public", reason }) if reason.contains("overflow")
        ));
    }
}
