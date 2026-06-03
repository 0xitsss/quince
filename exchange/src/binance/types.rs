use crate::r#trait::StreamMsg;
use quince_core::types::*;
use simd_json::prelude::*;
use simd_json::BorrowedValue;

pub fn parse_ws_msg(text: String) -> Option<StreamMsg> {
    let mut bytes = text.into_bytes();
    let val = simd_json::to_borrowed_value(&mut bytes).ok()?;

    let inner = if let Some(data) = val.get("data") { data } else { &val };

    let event = inner.get("e")?.as_str()?;

    match event {
        "aggTrade" | "trade" => {
            let trade = inner;
            Some(StreamMsg::Trade(Trade {
                price: trade.get("p")?.as_str()?.parse().ok()?,
                qty: trade.get("q")?.as_str()?.parse().ok()?,
                time: chrono::DateTime::from_timestamp_millis(trade.get("T")?.as_u64()? as i64)?,
                side: if trade.get("m")?.as_bool()? { Side::Sell } else { Side::Buy },
                trade_id: trade.get("t").or_else(|| trade.get("a"))?.as_u64()?,
            }))
        }
        "depthUpdate" | "depth" => {
            let bids = parse_depth_levels(inner.get("b")?.as_array()?);
            let asks = parse_depth_levels(inner.get("a")?.as_array()?);
            Some(StreamMsg::Depth(Depth { bids, asks }))
        }
        "markPriceUpdate" => {
            Some(StreamMsg::MarkPrice {
                price: inner.get("p")?.as_str()?.parse().ok()?,
                time: chrono::DateTime::from_timestamp_millis(inner.get("E")?.as_u64()? as i64)?,
            })
        }
        "openInterest" => {
            Some(StreamMsg::OpenInterest {
                qty: inner.get("o")?.as_str()?.parse().ok()?,
                time: chrono::DateTime::from_timestamp_millis(inner.get("E")?.as_u64()? as i64)?,
            })
        }
        "forceOrder" => {
            let order = inner.get("o")?;
            Some(StreamMsg::ForceOrder(Trade {
                price: order.get("ap")?.as_str()?.parse().ok()?,
                qty: order.get("z")?.as_str()?.parse().ok()?,
                time: chrono::DateTime::from_timestamp_millis(order.get("T")?.as_u64()? as i64)?,
                side: if order.get("S")?.as_str()? == "BUY" { Side::Buy } else { Side::Sell },
                trade_id: inner.get("E")?.as_u64()?,
            }))
        }
        "ORDER_TRADE_UPDATE" => {
            let order = inner.get("o")?;
            let side = if order.get("S")?.as_str()? == "BUY" { Side::Buy } else { Side::Sell };
            let filled_qty: f64 = order.get("l")?.as_str()?.parse().ok()?;
            if filled_qty > 0.0 {
                Some(StreamMsg::OrderUpdate(OrderFill {
                    order_id: order.get("i")?.as_u64().map(|v| v.to_string()).unwrap_or_default(),
                    side,
                    price: order.get("L")?.as_str()?.parse().ok()?,
                    qty: filled_qty,
                    fee: order.get("n")?.as_str().and_then(|s| s.parse().ok()).unwrap_or(0.0),
                    fee_asset: order.get("N")?.as_str()?.to_string(),
                    time: chrono::DateTime::from_timestamp_millis(order.get("T")?.as_u64()? as i64)?,
                }))
            } else {
                None
            }
        }
        "ACCOUNT_UPDATE" => {
            let acct = inner.get("a")?;
            let balances = parse_balances(acct.get("B")?.as_array()?);
            let positions = parse_positions(acct.get("P")?.as_array()?);
            Some(StreamMsg::AccountUpdate(AccountInfo { balances, positions }))
        }
        _ => None,
    }
}

fn parse_depth_levels(arr: &[BorrowedValue<'_>]) -> Vec<DepthLevel> {
    let mut out = Vec::with_capacity(arr.len());
    for level in arr {
        if let Some(entry) = level.as_array() {
            if entry.len() >= 2 {
                if let (Some(p), Some(q)) = (
                    entry[0].as_str().and_then(|s| s.parse().ok()),
                    entry[1].as_str().and_then(|s| s.parse().ok()),
                ) {
                    out.push(DepthLevel { price: p, qty: q });
                }
            }
        }
    }
    out
}

fn parse_balances(arr: &[BorrowedValue<'_>]) -> Vec<Balance> {
    let mut out = Vec::with_capacity(arr.len());
    for b in arr {
        out.push(Balance {
            asset: b.get("a").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            wallet: b.get("wb").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0),
            cross_wallet: b.get("cw").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0),
        });
    }
    out
}

fn parse_positions(arr: &[BorrowedValue<'_>]) -> Vec<Position> {
    let mut out = Vec::with_capacity(arr.len());
    for p in arr {
        let size: f64 = p.get("pa").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0);
        let side = if size > 0.0 {
            PositionSide::Long
        } else if size < 0.0 {
            PositionSide::Short
        } else {
            PositionSide::None
        };
        out.push(Position {
            symbol: p.get("s").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            side,
            size: size.abs(),
            entry_price: p.get("ep").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0),
            unrealized_pnl: p.get("up").and_then(|v| v.as_str()).and_then(|s| s.parse().ok()).unwrap_or(0.0),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::types::Side;

    #[test]
    fn parse_agg_trade() {
        let json = r#"{"e":"aggTrade","E":1717200000000,"s":"BTCUSDT","t":123456,"p":"48231.50","q":"0.001","b":12345,"a":67890,"T":1717200000000,"m":true}"#;
        let msg = parse_ws_msg(json.to_string()).unwrap();
        match msg {
            StreamMsg::Trade(t) => {
                assert!((t.price - 48231.50).abs() < 1e-10);
                assert!((t.qty - 0.001).abs() < 1e-10);
                assert_eq!(t.trade_id, 123456);
                assert_eq!(t.side, Side::Sell);
            }
            _ => panic!("expected Trade, got {:?}", msg),
        }
    }

    #[test]
    fn parse_depth20() {
        let json = r#"{"e":"depthUpdate","E":1717200000000,"s":"BTCUSDT","b":[["48231.50","1.234"],["48230.00","0.567"]],"a":[["48235.00","0.890"],["48236.50","1.111"]],"u":123456}"#;
        let msg = parse_ws_msg(json.to_string()).unwrap();
        match msg {
            StreamMsg::Depth(d) => {
                assert_eq!(d.bids.len(), 2);
                assert_eq!(d.asks.len(), 2);
                assert!((d.bids[0].price - 48231.50).abs() < 1e-10);
                assert!((d.bids[0].qty - 1.234).abs() < 1e-10);
                assert!((d.bids[1].price - 48230.00).abs() < 1e-10);
                assert!((d.bids[1].qty - 0.567).abs() < 1e-10);
                assert!((d.asks[0].price - 48235.00).abs() < 1e-10);
                assert!((d.asks[0].qty - 0.890).abs() < 1e-10);
                assert!((d.asks[1].price - 48236.50).abs() < 1e-10);
                assert!((d.asks[1].qty - 1.111).abs() < 1e-10);
            }
            _ => panic!("expected Depth, got {:?}", msg),
        }
    }

    #[test]
    fn parse_combined_stream() {
        let json = r#"{"stream":"btcusdt@aggTrade","data":{"e":"aggTrade","E":1717200000000,"s":"BTCUSDT","t":999,"p":"50000.00","q":"0.01","b":1,"a":2,"T":1717200000000,"m":false}}"#;
        let msg = parse_ws_msg(json.to_string()).unwrap();
        match msg {
            StreamMsg::Trade(t) => {
                assert!((t.price - 50000.00).abs() < 1e-10);
                assert!((t.qty - 0.01).abs() < 1e-10);
                assert_eq!(t.trade_id, 999);
                assert_eq!(t.side, Side::Buy);
            }
            _ => panic!("expected Trade, got {:?}", msg),
        }
    }

    #[test]
    fn parse_mark_price() {
        let json = r#"{"e":"markPriceUpdate","E":1717200000000,"s":"BTCUSDT","p":"48231.50","i":"48200.00","r":"0.0001","T":1717200000000}"#;
        let msg = parse_ws_msg(json.to_string()).unwrap();
        match msg {
            StreamMsg::MarkPrice { price, .. } => {
                assert!((price - 48231.50).abs() < 1e-10);
            }
            _ => panic!("expected MarkPrice, got {:?}", msg),
        }
    }

    #[test]
    fn parse_invalid_json() {
        assert!(parse_ws_msg("not json at all".to_string()).is_none());
    }

    #[test]
    fn parse_unknown_event() {
        let json = r#"{"e":"someRandomEvent","E":123}"#;
        assert!(parse_ws_msg(json.to_string()).is_none());
    }
}


