use crate::r#trait::StreamMsg;
use quince_core::types::*;
use serde::Deserialize;

pub fn parse_ws_msg(text: &str) -> Option<StreamMsg> {
    let val: serde_json::Value = serde_json::from_str(text).ok()?;
    let event = val["e"].as_str()?;

    match event {
        "aggTrade" | "trade" => {
            let t: WsTrade = serde_json::from_str(text).ok()?;
            Some(StreamMsg::Trade(Trade {
                price: t.p.parse().ok()?,
                qty: t.q.parse().ok()?,
                time: chrono::DateTime::from_timestamp_millis(t.T as i64)?,
                side: if t.m { Side::Sell } else { Side::Buy },
                trade_id: t.t,
            }))
        }
        "depthUpdate" | "depth" => {
            let d: WsDepth = serde_json::from_str(text).ok()?;
            let mut bids = Vec::with_capacity(d.b.len());
            for level in d.b {
                if let (Some(p), Some(q)) = (level[0].parse().ok(), level[1].parse().ok()) {
                    bids.push(DepthLevel { price: p, qty: q });
                }
            }
            let mut asks = Vec::with_capacity(d.a.len());
            for level in d.a {
                if let (Some(p), Some(q)) = (level[0].parse().ok(), level[1].parse().ok()) {
                    asks.push(DepthLevel { price: p, qty: q });
                }
            }
            Some(StreamMsg::Depth(Depth { bids, asks }))
        }
        "markPriceUpdate" => {
            let m: WsMarkPrice = serde_json::from_str(text).ok()?;
            Some(StreamMsg::MarkPrice {
                price: m.p.parse().ok()?,
                time: chrono::DateTime::from_timestamp_millis(m.E as i64)?,
            })
        }
        "openInterest" => {
            let oi: WsOpenInterest = serde_json::from_str(text).ok()?;
            Some(StreamMsg::OpenInterest {
                qty: oi.o.parse().ok()?,
                time: chrono::DateTime::from_timestamp_millis(oi.E as i64)?,
            })
        }
        "forceOrder" => {
            let fo: WsForceOrder = serde_json::from_str(text).ok()?;
            Some(StreamMsg::ForceOrder(Trade {
                price: fo.o.ap.parse().ok()?,
                qty: fo.o.z.parse().ok()?,
                time: chrono::DateTime::from_timestamp_millis(fo.o.T as i64)?,
                side: if fo.o.S == "BUY" { Side::Buy } else { Side::Sell },
                trade_id: fo.E,
            }))
        }
        "ORDER_TRADE_UPDATE" => {
            let o: WsOrderUpdate = serde_json::from_str(text).ok()?;
            let side = if o.o.S == "BUY" { Side::Buy } else { Side::Sell };
            let filled_qty: f64 = o.o.l.parse().ok()?;
            if filled_qty > 0.0 {
                Some(StreamMsg::OrderUpdate(OrderFill {
                    order_id: o.o.i.to_string(),
                    side,
                    price: o.o.L.parse().ok()?,
                    qty: filled_qty,
                    fee: o.o.n.parse().unwrap_or(0.0),
                    fee_asset: o.o.N.clone(),
                    time: chrono::DateTime::from_timestamp_millis(o.o.T as i64)?,
                }))
            } else {
                None
            }
        }
        "ACCOUNT_UPDATE" => {
            let a: WsAccountUpdate = serde_json::from_str(text).ok()?;
            let mut balances = Vec::with_capacity(a.a.B.len());
            for b in a.a.B {
                balances.push(Balance {
                    asset: b.a,
                    wallet: b.wb.parse().unwrap_or(0.0),
                    cross_wallet: b.cw.parse().unwrap_or(0.0),
                });
            }
            let mut positions = Vec::with_capacity(a.a.P.len());
            for p in a.a.P {
                let size: f64 = p.pa.parse().unwrap_or(0.0);
                let side = if size > 0.0 {
                    PositionSide::Long
                } else if size < 0.0 {
                    PositionSide::Short
                } else {
                    PositionSide::None
                };
                positions.push(Position {
                    symbol: p.s,
                    side,
                    size: size.abs(),
                    entry_price: p.ep.parse().unwrap_or(0.0),
                    unrealized_pnl: p.up.parse().unwrap_or(0.0),
                });
            }
            Some(StreamMsg::AccountUpdate(AccountInfo { balances, positions }))
        }
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct WsTrade {
    pub e: String,      // event type
    pub E: u64,         // event time
    pub s: String,      // symbol
    pub t: u64,         // trade id
    pub p: String,      // price
    pub q: String,      // qty
    pub b: u64,         // buyer order id
    pub a: u64,         // seller order id
    pub T: u64,         // trade time
    pub m: bool,         // is buyer market maker
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct WsDepth {
    pub e: String,
    pub E: u64,
    pub s: String,
    pub b: Vec<Vec<String>>,  // bids [[price, qty], ...]
    pub a: Vec<Vec<String>>,  // asks [[price, qty], ...]
    pub u: u64,               // update id
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct WsMarkPrice {
    pub e: String,
    pub E: u64,
    pub s: String,
    pub p: String,   // mark price
    pub i: String,   // index price
    pub r: String,   // funding rate
    pub T: u64,      // next funding time
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct WsOpenInterest {
    pub e: String,
    pub E: u64,
    pub s: String,
    pub o: String,   // open interest
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct WsForceOrder {
    pub e: String,
    pub E: u64,
    pub s: String,
    pub o: ForceOrderData,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct ForceOrderData {
    pub s: String,    // symbol
    pub S: String,    // side BUY/SELL
    pub o: String,    // order type
    pub f: String,    // time in force
    pub q: String,    // original qty
    pub p: String,    // price
    pub ap: String,   // avg price
    pub X: String,    // order status
    pub l: String,    // last filled qty
    pub z: String,    // cumulative filled qty
    pub T: u64,       // trade time
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct WsOrderUpdate {
    pub e: String,
    pub E: u64,
    pub o: WsOrderData,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct WsOrderData {
    pub s: String,
    pub c: String,
    pub S: String,
    pub o: String,
    pub q: String,
    pub p: String,
    pub ap: String,
    pub x: String,
    pub X: String,
    pub i: u64,
    pub l: String,
    pub z: String,
    pub L: String,
    pub n: String,
    pub N: String,
    pub T: u64,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct WsAccountUpdate {
    pub e: String,
    pub E: u64,
    pub a: WsAccountData,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct WsAccountData {
    pub B: Vec<WsBalance>,
    pub P: Vec<WsPosition>,
    pub m: String,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct WsBalance {
    pub a: String,
    pub wb: String,
    pub cw: String,
}

#[derive(Debug, Deserialize)]
#[allow(non_snake_case)]
pub struct WsPosition {
    pub s: String,
    pub pa: String,
    pub ep: String,
    pub cr: String,
    pub up: String,
    pub ps: String,
}
