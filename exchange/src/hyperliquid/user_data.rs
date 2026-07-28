// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Hyperliquid private user-stream decoding and supervision.
//!
//! The WebSocket subscriptions are read-only, but their payloads are part of
//! the execution integrity boundary. A malformed event, disconnect, or full
//! ingress queue therefore emits `ReconcileRequired` before reconnecting.

use crate::r#trait::{ExchangeError, Result as ExchangeResult, StreamMsg};
use futures_util::{SinkExt, StreamExt};
use quince_core::types::{AccountInfo, Balance, OrderFill, Position, PositionSide, Side};
use serde_json::{Map, Value};
use std::time::Duration;

const RECONNECT_DELAY: Duration = Duration::from_secs(1);
const USER_DATA_SOURCE: &str = "hyperliquid-user-data";

type UserDataSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn ws_url(testnet: bool) -> &'static str {
    if testnet {
        "wss://api.hyperliquid-testnet.xyz/ws"
    } else {
        "wss://api.hyperliquid.xyz/ws"
    }
}

fn signal(tx: &crossbeam_channel::Sender<StreamMsg>, reason: impl Into<String>) {
    let _ = tx.try_send(StreamMsg::ReconcileRequired {
        source: USER_DATA_SOURCE,
        reason: reason.into(),
    });
}

fn validate_address(address: &str) -> ExchangeResult<String> {
    let address = address.trim();
    if address.len() != 42
        || !address.starts_with("0x")
        || !address[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ExchangeError::Auth(
            "Hyperliquid user stream requires a 20-byte 0x-prefixed hexadecimal address".into(),
        ));
    }
    Ok(address.to_ascii_lowercase())
}

async fn connect_user_stream(testnet: bool, address: &str) -> ExchangeResult<UserDataSocket> {
    let (mut socket, _) = tokio_tungstenite::connect_async(ws_url(testnet))
        .await
        .map_err(|error| ExchangeError::Ws(format!("connect Hyperliquid user stream: {error}")))?;
    for subscription in ["userFills", "orderUpdates", "clearinghouseState"] {
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                serde_json::json!({
                    "method": "subscribe",
                    "subscription": { "type": subscription, "user": address },
                })
                .to_string(),
            ))
            .await
            .map_err(|error| {
                ExchangeError::Ws(format!(
                    "subscribe Hyperliquid {subscription} stream: {error}"
                ))
            })?;
    }
    Ok(socket)
}

async fn run_user_data_session(
    socket: UserDataSocket,
    tx: &crossbeam_channel::Sender<StreamMsg>,
) -> ExchangeResult<()> {
    let (mut writer, mut reader) = socket.split();
    loop {
        match reader.next().await {
            Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                let events = parse_user_data_msgs(&text).map_err(|error| {
                    ExchangeError::Ws(format!("invalid Hyperliquid user event: {error}"))
                })?;
                for event in events {
                    tx.try_send(event).map_err(|error| match error {
                        crossbeam_channel::TrySendError::Full(_) => {
                            ExchangeError::Ws("Hyperliquid user-data ingress overflow".into())
                        }
                        crossbeam_channel::TrySendError::Disconnected(_) => {
                            ExchangeError::Disconnected
                        }
                    })?;
                }
            }
            Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(payload))) => writer
                .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                .await
                .map_err(|error| ExchangeError::Ws(error.to_string()))?,
            Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) | None => {
                return Err(ExchangeError::Disconnected);
            }
            Some(Err(error)) => return Err(ExchangeError::Ws(error.to_string())),
            _ => {}
        }
    }
}

/// Starts a self-healing private user-data supervisor. A returned value means
/// the initial subscriptions are live; later gaps cause an immediate engine
/// reconciliation signal and bounded-delay reconnect.
pub async fn start_user_data_stream(
    address: String,
    testnet: bool,
    tx: crossbeam_channel::Sender<StreamMsg>,
) -> ExchangeResult<()> {
    let address = validate_address(&address)?;
    let initial_socket = connect_user_stream(testnet, &address).await?;
    tokio::spawn(async move {
        let mut next_socket = Some(initial_socket);
        loop {
            let session = match next_socket.take() {
                Some(socket) => run_user_data_session(socket, &tx).await,
                None => match connect_user_stream(testnet, &address).await {
                    Ok(socket) => run_user_data_session(socket, &tx).await,
                    Err(error) => Err(error),
                },
            };
            if let Err(error) = session {
                signal(&tx, format!("private stream gap: {error}"));
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    });
    Ok(())
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UserDataParseError {
    #[error("invalid JSON: {0}")]
    Json(String),
    #[error("{0}")]
    Invalid(&'static str),
}

pub type ParseResult<T> = std::result::Result<T, UserDataParseError>;

/// Parses every lossless engine event in one private WebSocket payload.
/// `userFills` may contain multiple fills; dropping all but the first would
/// silently understate fee and position accounting.
pub fn parse_user_data_msgs(text: &str) -> ParseResult<Vec<StreamMsg>> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| UserDataParseError::Json(error.to_string()))?;
    let root = object(&value, "user event must be a JSON object")?;
    let channel = required_str(root, "channel", "user event channel is missing or invalid")?;
    let data = required(root, "data", "user event data is missing")?;
    match channel {
        "userFills" => parse_user_fills(object(data, "user-fills data is invalid")?),
        "orderUpdates" => parse_order_updates(data),
        "clearinghouseState" => {
            parse_account_update(object(data, "clearinghouse-state data is invalid")?)
                .map(|account| vec![StreamMsg::AccountUpdate(account)])
        }
        "subscriptionResponse" => Ok(Vec::new()),
        _ => Ok(Vec::new()),
    }
}

fn parse_user_fills(data: &Map<String, Value>) -> ParseResult<Vec<StreamMsg>> {
    required_str(data, "user", "user-fills user is missing or invalid")?;
    let fills = array(
        required(data, "fills", "user-fills payload is missing fills")?,
        "user-fills fills is invalid",
    )?;
    fills
        .iter()
        .map(parse_fill)
        .map(|fill| fill.map(StreamMsg::OrderUpdate))
        .collect()
}

fn parse_fill(value: &Value) -> ParseResult<OrderFill> {
    let fill = object(value, "fill is invalid")?;
    let order_id = required_u64(fill, "oid", "fill order id is missing or invalid")?;
    if order_id == 0 {
        return Err(UserDataParseError::Invalid(
            "fill order id must be non-zero",
        ));
    }
    let qty = positive_number(
        required_str(fill, "sz", "fill size is missing or invalid")?,
        "fill size must be positive and finite",
    )?;
    let price = positive_number(
        required_str(fill, "px", "fill price is missing or invalid")?,
        "fill price must be positive and finite",
    )?;
    let fee = finite_number(
        required_str(fill, "fee", "fill fee is missing or invalid")?,
        "fill fee must be finite",
    )?;
    let fee_asset = required_str(fill, "feeToken", "fill fee token is missing or invalid")?;
    if fee_asset.trim().is_empty() {
        return Err(UserDataParseError::Invalid(
            "fill fee token must not be empty",
        ));
    }
    Ok(OrderFill {
        order_id: order_id.to_string(),
        side: parse_side(required_str(
            fill,
            "side",
            "fill side is missing or invalid",
        )?)?,
        price,
        qty,
        fee,
        fee_asset: fee_asset.to_owned(),
        time: timestamp(fill, "time", "fill time is missing or invalid")?,
    })
}

fn parse_order_updates(value: &Value) -> ParseResult<Vec<StreamMsg>> {
    let updates = array(value, "order-updates data is invalid")?;
    updates
        .iter()
        .map(|value| {
            let update = object(value, "order update is invalid")?;
            let order = object(
                required(update, "order", "order update is missing order")?,
                "order update order is invalid",
            )?;
            let oid = required_u64(order, "oid", "order update id is missing or invalid")?;
            if oid == 0 {
                return Err(UserDataParseError::Invalid(
                    "order update id must be non-zero",
                ));
            }
            let status = required_str(
                update,
                "status",
                "order update status is missing or invalid",
            )?;
            if status.trim().is_empty() {
                return Err(UserDataParseError::Invalid(
                    "order update status must not be empty",
                ));
            }
            Ok(StreamMsg::ReconcileRequired {
                source: USER_DATA_SOURCE,
                reason: format!("order {oid} updated with status {status}"),
            })
        })
        .collect()
}

fn parse_account_update(data: &Map<String, Value>) -> ParseResult<AccountInfo> {
    let summary = object(
        required(
            data,
            "crossMarginSummary",
            "account update is missing cross margin summary",
        )?,
        "cross margin summary is invalid",
    )?;
    let account_value = nonnegative_number(
        required_str(
            summary,
            "accountValue",
            "account value is missing or invalid",
        )?,
        "account value must be non-negative and finite",
    )?;
    let positions = array(
        required(
            data,
            "assetPositions",
            "account update is missing asset positions",
        )?,
        "asset positions is invalid",
    )?
    .iter()
    .map(parse_position)
    .collect::<ParseResult<Vec<_>>>()?;
    Ok(AccountInfo {
        balances: vec![Balance {
            asset: "USDC".into(),
            wallet: account_value,
            cross_wallet: account_value,
        }],
        positions,
    })
}

fn parse_position(value: &Value) -> ParseResult<Position> {
    let entry = object(value, "asset position is invalid")?;
    let position = object(
        required(entry, "position", "asset position is missing position")?,
        "asset position position is invalid",
    )?;
    let signed_size = finite_number(
        required_str(position, "szi", "position size is missing or invalid")?,
        "position size must be finite",
    )?;
    let entry_price = match position.get("entryPx") {
        None | Some(Value::Null) => 0.0,
        Some(value) => positive_number(
            value.as_str().ok_or(UserDataParseError::Invalid(
                "position entry price is invalid",
            ))?,
            "position entry price must be positive and finite",
        )?,
    };
    let symbol = required_str(position, "coin", "position coin is missing or invalid")?;
    if symbol.trim().is_empty() {
        return Err(UserDataParseError::Invalid(
            "position coin must not be empty",
        ));
    }
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
        unrealized_pnl: finite_number(
            required_str(
                position,
                "unrealizedPnl",
                "position unrealized PnL is missing or invalid",
            )?,
            "position unrealized PnL must be finite",
        )?,
    })
}

fn object<'a>(value: &'a Value, message: &'static str) -> ParseResult<&'a Map<String, Value>> {
    value
        .as_object()
        .ok_or(UserDataParseError::Invalid(message))
}
fn array<'a>(value: &'a Value, message: &'static str) -> ParseResult<&'a Vec<Value>> {
    value.as_array().ok_or(UserDataParseError::Invalid(message))
}
fn required<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    message: &'static str,
) -> ParseResult<&'a Value> {
    object
        .get(field)
        .ok_or(UserDataParseError::Invalid(message))
}
fn required_str<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    message: &'static str,
) -> ParseResult<&'a str> {
    required(object, field, message)?
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or(UserDataParseError::Invalid(message))
}
fn required_u64(
    object: &Map<String, Value>,
    field: &str,
    message: &'static str,
) -> ParseResult<u64> {
    required(object, field, message)?
        .as_u64()
        .ok_or(UserDataParseError::Invalid(message))
}
fn finite_number(value: &str, message: &'static str) -> ParseResult<f64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or(UserDataParseError::Invalid(message))
}
fn positive_number(value: &str, message: &'static str) -> ParseResult<f64> {
    finite_number(value, message).and_then(|value| {
        if value > 0.0 {
            Ok(value)
        } else {
            Err(UserDataParseError::Invalid(message))
        }
    })
}
fn nonnegative_number(value: &str, message: &'static str) -> ParseResult<f64> {
    finite_number(value, message).and_then(|value| {
        if value >= 0.0 {
            Ok(value)
        } else {
            Err(UserDataParseError::Invalid(message))
        }
    })
}
fn parse_side(value: &str) -> ParseResult<Side> {
    match value {
        "B" | "buy" => Ok(Side::Buy),
        "A" | "sell" => Ok(Side::Sell),
        _ => Err(UserDataParseError::Invalid("side must be B/buy or A/sell")),
    }
}
fn timestamp(
    object: &Map<String, Value>,
    field: &str,
    message: &'static str,
) -> ParseResult<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::from_timestamp_millis(
        required(object, field, message)?
            .as_i64()
            .ok_or(UserDataParseError::Invalid(message))?,
    )
    .ok_or(UserDataParseError::Invalid(message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_fills_strictly() {
        let event = parse_user_data_msgs(
            r#"{"channel":"userFills","data":{"isSnapshot":false,"user":"0x0123456789abcdef0123456789abcdef01234567","fills":[{"coin":"BTC","px":"65000","sz":"0.1","side":"B","time":1717200000000,"oid":42,"fee":"0.5","feeToken":"USDC"}]}}"#,
        )
        .unwrap()
        .pop()
        .unwrap();
        assert!(
            matches!(event, StreamMsg::OrderUpdate(OrderFill { order_id, side: Side::Buy, price, qty, fee, ref fee_asset, .. }) if order_id == "42" && price == 65000.0 && qty == 0.1 && fee == 0.5 && fee_asset == "USDC")
        );
    }

    #[test]
    fn rejects_malformed_user_fill_without_mutating_accounting() {
        let error = parse_user_data_msgs(
            r#"{"channel":"userFills","data":{"user":"0xabc","fills":[{"coin":"BTC","px":"NaN","sz":"0.1","side":"B","time":1717200000000,"oid":42,"fee":"0","feeToken":"USDC"}]}}"#,
        )
        .expect_err("non-finite price must not reach the engine");
        assert!(error.to_string().contains("fill price"));
    }

    #[test]
    fn order_update_requires_authoritative_reconciliation() {
        let event = parse_user_data_msgs(
            r#"{"channel":"orderUpdates","data":[{"order":{"oid":42,"coin":"BTC","side":"B","sz":"0.1","limitPx":"65000"},"status":"canceled"}]}"#,
        )
        .unwrap()
        .pop()
        .unwrap();
        assert!(
            matches!(event, StreamMsg::ReconcileRequired { source: "hyperliquid-user-data", reason } if reason.contains("42") && reason.contains("canceled"))
        );
    }

    #[test]
    fn parses_clearinghouse_account_snapshot() {
        let event = parse_user_data_msgs(
            r#"{"channel":"clearinghouseState","data":{"crossMarginSummary":{"accountValue":"123.45"},"assetPositions":[{"position":{"coin":"BTC","szi":"-0.1","entryPx":"65000","unrealizedPnl":"2"}}]}}"#,
        )
        .unwrap()
        .pop()
        .unwrap();
        assert!(
            matches!(event, StreamMsg::AccountUpdate(AccountInfo { balances, positions }) if balances[0].asset == "USDC" && balances[0].wallet == 123.45 && positions[0].side == PositionSide::Short)
        );
    }
}
