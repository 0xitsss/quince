use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy)]
pub struct Trade {
    pub price: f64,
    pub qty: f64,
    pub time: DateTime<Utc>,
    pub side: Side,
    pub trade_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn from_taker(side: &str) -> Self {
        match side {
            "buy" | "BUY" | "takerBuy" => Side::Buy,
            _ => Side::Sell,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DepthLevel {
    pub price: f64,
    pub qty: f64,
}

#[derive(Debug, Clone)]
pub struct Depth {
    pub bids: Vec<DepthLevel>,
    pub asks: Vec<DepthLevel>,
}

#[derive(Debug, Clone)]
pub struct Order {
    pub symbol: String,
    pub side: Side,
    pub qty: f64,
    pub price: Option<f64>,
    pub order_type: OrderType,
    pub reduce_only: bool,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderType {
    Market,
    Limit,
}

#[derive(Debug, Clone)]
pub struct OrderFill {
    pub order_id: String,
    pub side: Side,
    pub price: f64,
    pub qty: f64,
    pub fee: f64,
    pub fee_asset: String,
    pub time: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub balances: Vec<Balance>,
    pub positions: Vec<Position>,
}

#[derive(Debug, Clone)]
pub struct Balance {
    pub asset: String,
    pub wallet: f64,
    pub cross_wallet: f64,
}

#[derive(Debug, Clone)]
pub struct Position {
    pub symbol: String,
    pub side: PositionSide,
    pub size: f64,
    pub entry_price: f64,
    pub unrealized_pnl: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PositionSide {
    Long,
    Short,
    None,
}
