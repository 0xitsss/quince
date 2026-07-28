// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Strict parser for Binance USDⓈ-M Futures user-data events.
//!
//! It owns the listen-key lifecycle as well as strict payload decoding. Socket
//! producers use a bounded crossbeam ingress and never wait for the engine.

use crate::r#trait::{ExchangeError, Result as ExchangeResult, StreamMsg};
use futures_util::{SinkExt, StreamExt};
use quince_core::types::{AccountInfo, Balance, OrderFill, Position, PositionSide, Side};
use serde_json::{Map, Value};
use std::time::Duration;

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30 * 60);
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

type UserDataSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn listen_key_url(testnet: bool) -> &'static str {
    if testnet {
        "https://testnet.binancefuture.com/fapi/v1/listenKey"
    } else {
        "https://fapi.binance.com/fapi/v1/listenKey"
    }
}

fn stream_url(testnet: bool, listen_key: &str) -> String {
    let base = if testnet {
        "wss://stream.binancefuture.com/ws"
    } else {
        "wss://fstream.binance.com/ws"
    };
    format!("{base}/{listen_key}")
}

async fn create_listen_key(
    client: &reqwest::Client,
    api_key: &str,
    testnet: bool,
) -> ExchangeResult<String> {
    let response = client
        .post(listen_key_url(testnet))
        .header("X-MBX-APIKEY", api_key)
        .send()
        .await
        .map_err(|e| ExchangeError::Rest(format!("start Binance user stream: {e}")))?
        .error_for_status()
        .map_err(|e| ExchangeError::Rest(format!("start Binance user stream status: {e}")))?;
    let value: Value = response
        .json()
        .await
        .map_err(|e| ExchangeError::Rest(format!("parse Binance listen key: {e}")))?;
    value["listenKey"]
        .as_str()
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ExchangeError::Rest("Binance listen-key response is missing listenKey".into())
        })
}

async fn keepalive(
    client: &reqwest::Client,
    api_key: &str,
    testnet: bool,
    listen_key: &str,
) -> ExchangeResult<()> {
    client
        .put(listen_key_url(testnet))
        .header("X-MBX-APIKEY", api_key)
        .query(&[("listenKey", listen_key)])
        .send()
        .await
        .map_err(|e| ExchangeError::Rest(format!("keepalive Binance user stream: {e}")))?
        .error_for_status()
        .map_err(|e| ExchangeError::Rest(format!("keepalive Binance user stream status: {e}")))?;
    Ok(())
}

fn signal(tx: &crossbeam_channel::Sender<StreamMsg>, reason: impl Into<String>) {
    let _ = tx.try_send(StreamMsg::ReconcileRequired {
        source: "binance-user-data",
        reason: reason.into(),
    });
}

async fn connect_user_stream(testnet: bool, listen_key: &str) -> ExchangeResult<UserDataSocket> {
    let (socket, _) = tokio_tungstenite::connect_async(stream_url(testnet, listen_key))
        .await
        .map_err(|e| ExchangeError::Ws(format!("connect Binance user stream: {e}")))?;
    Ok(socket)
}

async fn run_user_data_session(
    socket: UserDataSocket,
    client: &reqwest::Client,
    api_key: &str,
    testnet: bool,
    listen_key: &str,
    tx: &crossbeam_channel::Sender<StreamMsg>,
) -> ExchangeResult<()> {
    let (mut writer, mut reader) = socket.split();
    let mut keepalive_tick = tokio::time::interval(KEEPALIVE_INTERVAL);
    keepalive_tick.tick().await;
    loop {
        tokio::select! {
            _ = keepalive_tick.tick() => keepalive(client, api_key, testnet, listen_key).await?,
            message = reader.next() => match message {
                Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => match parse_user_data_msg(&text) {
                    Ok(Some(event)) => if tx.try_send(event).is_err() { return Err(ExchangeError::Ws("user-data ingress overflow".into())); },
                    Ok(None) => {
                        if text.contains("listenKeyExpired") { return Err(ExchangeError::Ws("Binance listen key expired".into())); }
                    }
                    Err(error) => return Err(ExchangeError::Ws(format!("invalid Binance user event: {error}"))),
                },
                Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(payload))) => writer.send(tokio_tungstenite::tungstenite::Message::Pong(payload)).await.map_err(|e| ExchangeError::Ws(e.to_string()))?,
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | None => return Err(ExchangeError::Disconnected),
                Some(Err(error)) => return Err(ExchangeError::Ws(error.to_string())),
                _ => {}
            }
        }
    }
}

/// Starts a self-healing private-stream supervisor. Every disconnect, parser
/// error, queue overflow, or listen-key failure emits `ReconcileRequired`
/// before reconnecting. Thus a transient stream gap never becomes invisible.
pub async fn start_user_data_stream(
    api_key: String,
    testnet: bool,
    tx: crossbeam_channel::Sender<StreamMsg>,
) -> ExchangeResult<()> {
    // A returned subscription means both the listen key and the private socket
    // are live. Starting the engine before that boundary leaves a window where
    // orders can be submitted without authoritative account/order updates.
    let initial_client = reqwest::Client::new();
    let initial_listen_key = create_listen_key(&initial_client, &api_key, testnet).await?;
    let initial_socket = connect_user_stream(testnet, &initial_listen_key).await?;
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut next_listen_key = Some(initial_listen_key);
        let mut next_socket = Some(initial_socket);
        loop {
            let listen_key = match next_listen_key.take() {
                Some(key) => key,
                None => match create_listen_key(&client, &api_key, testnet).await {
                    Ok(key) => key,
                    Err(error) => {
                        signal(&tx, format!("listen-key creation failed: {error}"));
                        tokio::time::sleep(RECONNECT_DELAY).await;
                        continue;
                    }
                },
            };
            let socket = match next_socket.take() {
                Some(socket) => Ok(socket),
                None => connect_user_stream(testnet, &listen_key).await,
            };
            let session = match socket {
                Ok(socket) => {
                    run_user_data_session(socket, &client, &api_key, testnet, &listen_key, &tx)
                        .await
                }
                Err(error) => Err(error),
            };
            if let Err(error) = session {
                signal(&tx, format!("private stream gap: {error}"));
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    });
    Ok(())
}

/// A malformed event must not be allowed to silently alter risk/accounting
/// state. Unknown event names are deliberately returned as `Ok(None)` so a
/// future Binance addition does not take down the stream by itself.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UserDataParseError {
    #[error("invalid JSON: {0}")]
    Json(String),
    #[error("{0}")]
    Invalid(&'static str),
}

pub type Result<T> = std::result::Result<T, UserDataParseError>;

/// Parses `ORDER_TRADE_UPDATE` and `ACCOUNT_UPDATE` payloads from the Binance
/// USDⓈ-M Futures user-data stream.
///
/// `ORDER_TRADE_UPDATE` produces `OrderUpdate` only for an actual `TRADE`
/// execution with positive last-fill quantity. Other valid order lifecycle
/// events have no corresponding lossless `StreamMsg` variant and return
/// `Ok(None)`; they must still be consumed by a future order-status/reconcile
/// layer rather than being mistaken for fills.
pub fn parse_user_data_msg(text: &str) -> Result<Option<StreamMsg>> {
    let root: Value =
        serde_json::from_str(text).map_err(|error| UserDataParseError::Json(error.to_string()))?;
    let root = object(&root, "event must be a JSON object")?;
    let event = required_str(root, "e", "event name `e` is missing or invalid")?;

    match event {
        "ORDER_TRADE_UPDATE" => parse_order_trade_update(root),
        "ACCOUNT_UPDATE" => {
            parse_account_update(root).map(|info| Some(StreamMsg::AccountUpdate(info)))
        }
        _ => Ok(None),
    }
}

fn parse_order_trade_update(root: &Map<String, Value>) -> Result<Option<StreamMsg>> {
    // Binance event/transaction timestamps are required event integrity data,
    // even though OrderFill only exposes the order transaction timestamp.
    timestamp(root, "E", "event time `E` is missing or invalid")?;
    timestamp(root, "T", "transaction time `T` is missing or invalid")?;
    let order = object(
        required(root, "o", "order payload `o` is missing")?,
        "order payload `o` is invalid",
    )?;

    let execution = required_str(order, "x", "execution type `o.x` is missing or invalid")?;
    if execution.is_empty() {
        return Err(UserDataParseError::Invalid(
            "execution type `o.x` must not be empty",
        ));
    }
    parse_side(required_str(
        order,
        "S",
        "order side `o.S` is missing or invalid",
    )?)?;
    let order_id = required_u64(order, "i", "order id `o.i` is missing or invalid")?;
    if order_id == 0 {
        return Err(UserDataParseError::Invalid(
            "order id `o.i` must be non-zero",
        ));
    }
    timestamp(
        order,
        "T",
        "order transaction time `o.T` is missing or invalid",
    )?;
    // Non-trade order transitions cannot be represented by OrderFill. Still
    // validate the envelope above, then intentionally emit no fill.
    if execution != "TRADE" {
        return Ok(None);
    }

    let side = parse_side(required_str(
        order,
        "S",
        "order side `o.S` is missing or invalid",
    )?)?;
    let order_id = order_id.to_string();
    let qty = positive_number(
        required_str(order, "l", "last fill quantity `o.l` is missing or invalid")?,
        "last fill quantity `o.l` must be positive and finite",
    )?;
    let price = positive_number(
        required_str(order, "L", "last fill price `o.L` is missing or invalid")?,
        "last fill price `o.L` must be positive and finite",
    )?;
    let fee = nonnegative_number(
        required_str(order, "n", "commission `o.n` is missing or invalid")?,
        "commission `o.n` must be non-negative and finite",
    )?;
    let fee_asset =
        required_str(order, "N", "commission asset `o.N` is missing or invalid")?.to_owned();
    if fee_asset.trim().is_empty() {
        return Err(UserDataParseError::Invalid(
            "commission asset `o.N` must not be empty",
        ));
    }
    let time = timestamp(
        order,
        "T",
        "order transaction time `o.T` is missing or invalid",
    )?;

    Ok(Some(StreamMsg::OrderUpdate(OrderFill {
        order_id,
        side,
        price,
        qty,
        fee,
        fee_asset,
        time,
    })))
}

fn parse_account_update(root: &Map<String, Value>) -> Result<AccountInfo> {
    timestamp(root, "E", "event time `E` is missing or invalid")?;
    timestamp(root, "T", "transaction time `T` is missing or invalid")?;
    let account = object(
        required(root, "a", "account payload `a` is missing")?,
        "account payload `a` is invalid",
    )?;
    required_str(
        account,
        "m",
        "account update reason `a.m` is missing or invalid",
    )?;
    let balances = array(
        required(account, "B", "balances `a.B` are missing")?,
        "balances `a.B` are invalid",
    )?
    .iter()
    .map(parse_balance)
    .collect::<Result<Vec<_>>>()?;
    let positions = array(
        required(account, "P", "positions `a.P` are missing")?,
        "positions `a.P` are invalid",
    )?
    .iter()
    .map(parse_position)
    .collect::<Result<Vec<_>>>()?;
    Ok(AccountInfo {
        balances,
        positions,
    })
}

fn parse_balance(value: &Value) -> Result<Balance> {
    let balance = object(value, "balance entry is invalid")?;
    let asset = required_str(balance, "a", "balance asset `a` is missing or invalid")?.to_owned();
    if asset.trim().is_empty() {
        return Err(UserDataParseError::Invalid(
            "balance asset `a` must not be empty",
        ));
    }
    Ok(Balance {
        asset,
        wallet: finite_number(
            required_str(balance, "wb", "wallet balance `wb` is missing or invalid")?,
            "wallet balance `wb` must be finite",
        )?,
        cross_wallet: finite_number(
            required_str(
                balance,
                "cw",
                "cross wallet balance `cw` is missing or invalid",
            )?,
            "cross wallet balance `cw` must be finite",
        )?,
    })
}

fn parse_position(value: &Value) -> Result<Position> {
    let position = object(value, "position entry is invalid")?;
    let symbol =
        required_str(position, "s", "position symbol `s` is missing or invalid")?.to_owned();
    if symbol.trim().is_empty() {
        return Err(UserDataParseError::Invalid(
            "position symbol `s` must not be empty",
        ));
    }
    let signed_size = finite_number(
        required_str(position, "pa", "position amount `pa` is missing or invalid")?,
        "position amount `pa` must be finite",
    )?;
    let side = if signed_size > 0.0 {
        PositionSide::Long
    } else if signed_size < 0.0 {
        PositionSide::Short
    } else {
        PositionSide::None
    };
    Ok(Position {
        symbol,
        side,
        size: signed_size.abs(),
        entry_price: nonnegative_number(
            required_str(position, "ep", "entry price `ep` is missing or invalid")?,
            "entry price `ep` must be non-negative and finite",
        )?,
        unrealized_pnl: finite_number(
            required_str(position, "up", "unrealized PnL `up` is missing or invalid")?,
            "unrealized PnL `up` must be finite",
        )?,
    })
}

fn parse_side(side: &str) -> Result<Side> {
    match side {
        "BUY" => Ok(Side::Buy),
        "SELL" => Ok(Side::Sell),
        _ => Err(UserDataParseError::Invalid(
            "order side `o.S` must be BUY or SELL",
        )),
    }
}

fn object<'a>(value: &'a Value, message: &'static str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or(UserDataParseError::Invalid(message))
}
fn array<'a>(value: &'a Value, message: &'static str) -> Result<&'a Vec<Value>> {
    value.as_array().ok_or(UserDataParseError::Invalid(message))
}
fn required<'a>(
    map: &'a Map<String, Value>,
    key: &str,
    message: &'static str,
) -> Result<&'a Value> {
    map.get(key).ok_or(UserDataParseError::Invalid(message))
}
fn required_str<'a>(
    map: &'a Map<String, Value>,
    key: &str,
    message: &'static str,
) -> Result<&'a str> {
    required(map, key, message)?
        .as_str()
        .ok_or(UserDataParseError::Invalid(message))
}
fn required_u64(map: &Map<String, Value>, key: &str, message: &'static str) -> Result<u64> {
    required(map, key, message)?
        .as_u64()
        .ok_or(UserDataParseError::Invalid(message))
}
fn finite_number(text: &str, message: &'static str) -> Result<f64> {
    let value = text
        .parse::<f64>()
        .map_err(|_| UserDataParseError::Invalid(message))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(UserDataParseError::Invalid(message))
    }
}
fn positive_number(text: &str, message: &'static str) -> Result<f64> {
    let value = finite_number(text, message)?;
    if value > 0.0 {
        Ok(value)
    } else {
        Err(UserDataParseError::Invalid(message))
    }
}
fn nonnegative_number(text: &str, message: &'static str) -> Result<f64> {
    let value = finite_number(text, message)?;
    if value >= 0.0 {
        Ok(value)
    } else {
        Err(UserDataParseError::Invalid(message))
    }
}
fn timestamp(
    map: &Map<String, Value>,
    key: &str,
    message: &'static str,
) -> Result<chrono::DateTime<chrono::Utc>> {
    let millis = required_u64(map, key, message)?;
    let millis = i64::try_from(millis).map_err(|_| UserDataParseError::Invalid(message))?;
    chrono::DateTime::from_timestamp_millis(millis).ok_or(UserDataParseError::Invalid(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDER_TRADE: &str = r#"{"e":"ORDER_TRADE_UPDATE","E":1568879465651,"T":1568879465650,"o":{"s":"BTCUSDT","c":"quince-1","S":"BUY","o":"LIMIT","f":"GTC","q":"0.001","p":"11794.15","ap":"11784.62","sp":"0","x":"TRADE","X":"PARTIALLY_FILLED","i":8886774,"l":"0.001","z":"0.001","L":"11784.62","n":"0.001","N":"USDT","T":1568879465650}}"#;
    const ACCOUNT_UPDATE: &str = r#"{"e":"ACCOUNT_UPDATE","E":1564745798939,"T":1564745798938,"a":{"m":"ORDER","B":[{"a":"USDT","wb":"122624.12345678","cw":"100.12345678"}],"P":[{"s":"BTCUSDT","pa":"-0.001","ep":"11784.62","up":"-0.12"}]}}"#;

    #[test]
    fn parses_trade_execution_strictly() {
        let Some(StreamMsg::OrderUpdate(fill)) = parse_user_data_msg(ORDER_TRADE).unwrap() else {
            panic!("expected order update")
        };
        assert_eq!(fill.order_id, "8886774");
        assert_eq!(fill.side, Side::Buy);
        assert_eq!(fill.fee_asset, "USDT");
        assert_eq!(fill.qty, 0.001);
    }

    #[test]
    fn parses_account_update_strictly() {
        let Some(StreamMsg::AccountUpdate(account)) = parse_user_data_msg(ACCOUNT_UPDATE).unwrap()
        else {
            panic!("expected account update")
        };
        assert_eq!(account.balances.len(), 1);
        assert_eq!(account.positions[0].side, PositionSide::Short);
        assert_eq!(account.positions[0].size, 0.001);
    }

    #[test]
    fn rejects_missing_or_invalid_required_values() {
        for event in [
            r#"{"e":"ORDER_TRADE_UPDATE","E":1,"T":1,"o":{"x":"TRADE"}}"#,
            r#"{"e":"ORDER_TRADE_UPDATE","E":1,"T":1,"o":{"S":"BUY","x":"TRADE","i":1,"l":"NaN","L":"1","n":"0","N":"USDT","T":1}}"#,
            r#"{"e":"ACCOUNT_UPDATE","E":1,"T":1,"a":{"m":"ORDER","B":[{"a":"USDT","wb":"x","cw":"0"}],"P":[]}}"#,
        ] {
            assert!(parse_user_data_msg(event).is_err(), "{event}");
        }
    }

    #[test]
    fn unknown_or_non_trade_order_events_are_not_misrepresented_as_fills() {
        assert!(parse_user_data_msg(r#"{"e":"listenKeyExpired"}"#)
            .unwrap()
            .is_none());
        let mut event: Value = serde_json::from_str(ORDER_TRADE).unwrap();
        event["o"]["x"] = Value::String("NEW".into());
        assert!(parse_user_data_msg(&event.to_string()).unwrap().is_none());
    }

    #[test]
    fn uses_futures_user_stream_endpoints() {
        assert_eq!(
            listen_key_url(false),
            "https://fapi.binance.com/fapi/v1/listenKey"
        );
        assert_eq!(
            listen_key_url(true),
            "https://testnet.binancefuture.com/fapi/v1/listenKey"
        );
        assert_eq!(stream_url(false, "key"), "wss://fstream.binance.com/ws/key");
        assert_eq!(
            stream_url(true, "key"),
            "wss://stream.binancefuture.com/ws/key"
        );
    }
}
