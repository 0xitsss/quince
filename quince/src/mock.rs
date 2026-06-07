use chrono::Utc;
use crossbeam_channel;
use quince::core::types::*;
use quince::exchange::binance::public::BinancePublic;
use quince::exchange::r#trait::{Exchange, OrderStatus, Result, Stream, StreamMsg};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct MockState {
    stream_tx: Option<crossbeam_channel::Sender<StreamMsg>>,
    positions: Vec<Position>,
    balances: Vec<Balance>,
    last_price: f64,
}

pub struct MockExchange {
    order_counter: AtomicU64,
    public: Option<BinancePublic>,
    state: Arc<Mutex<MockState>>,
}

impl MockExchange {
    fn new_state() -> Arc<Mutex<MockState>> {
        Arc::new(Mutex::new(MockState {
            stream_tx: None,
            positions: vec![],
            last_price: 100.0,
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
        }))
    }

    pub fn new() -> Self {
        Self {
            order_counter: AtomicU64::new(1),
            public: None,
            state: Self::new_state(),
        }
    }

    pub fn new_public() -> Self {
        Self {
            order_counter: AtomicU64::new(1),
            public: Some(BinancePublic::new()),
            state: Self::new_state(),
        }
    }
}

#[async_trait::async_trait]
impl Exchange for MockExchange {
    async fn subscribe(&self, symbols: &[String]) -> Result<Stream> {
        if let Some(ref public) = self.public {
            let public_stream = public.subscribe(symbols).await?;
            let (tx, rx) = crossbeam_channel::bounded(1024);
            let state = Arc::clone(&self.state);
            state.lock().unwrap().stream_tx = Some(tx.clone());
            // Forward real WS data + track price + inject our own events
            std::thread::spawn(move || {
                while let Ok(msg) = public_stream.rx.recv() {
                    match &msg {
                        StreamMsg::Trade(t) | StreamMsg::ForceOrder(t) => {
                            state.lock().unwrap().last_price = t.price;
                        }
                        StreamMsg::MarkPrice { price, .. } => {
                            state.lock().unwrap().last_price = *price;
                        }
                        _ => {}
                    }
                    if tx.try_send(msg).is_err() {
                        break;
                    }
                }
            });
            return Ok(Stream { rx });
        }
        let (tx, rx) = crossbeam_channel::bounded(1024);
        let state = Arc::clone(&self.state);
        state.lock().unwrap().stream_tx = Some(tx.clone());

        std::thread::spawn(move || {
            let mut tick = 0u64;
            let mut depth_tick = 0u64;
            loop {
                let phase = (tick as f64) * 0.01;
                let price = 100.0 + 10.0 * phase.sin();
                state.lock().unwrap().last_price = price;
                let qty = 0.1 + 0.05 * (phase * 3.0).sin();
                let side = if (phase * 2.0).sin() > 0.0 {
                    Side::Buy
                } else {
                    Side::Sell
                };
                let ts = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as i64;

                let trade = Trade {
                    price,
                    qty,
                    time: chrono::DateTime::from_timestamp_millis(ts).unwrap(),
                    side,
                    trade_id: tick,
                };
                tracing::debug!(target: "mock", "StreamMsg::Trade price={price} qty={qty} id={tick}");
                let _ = tx.try_send(StreamMsg::Trade(trade));

                if depth_tick % 5 == 0 {
                    let spread = price * 0.001;
                    let bids: Vec<DepthLevel> = (0..10)
                        .map(|i| DepthLevel {
                            price: price - spread * (i + 1) as f64,
                            qty: 1.0 - i as f64 * 0.1,
                        })
                        .collect();
                    let asks: Vec<DepthLevel> = (0..10)
                        .map(|i| DepthLevel {
                            price: price + spread * (i + 1) as f64,
                            qty: 1.0 - i as f64 * 0.1,
                        })
                        .collect();
                    tracing::debug!(target: "mock", "StreamMsg::Depth bids={} asks={}", bids.len(), asks.len());
                    let _ = tx.try_send(StreamMsg::Depth(Depth { bids, asks }));
                }

                tick += 1;
                depth_tick += 1;
                std::thread::sleep(Duration::from_millis(10));

                if tick > 5000 {
                    break;
                }
            }
        });

        Ok(Stream { rx })
    }

    async fn place_order(&self, order: Order) -> Result<String> {
        let id = format!(
            "mock_{}",
            self.order_counter.fetch_add(1, Ordering::Relaxed)
        );
        let mut state = self.state.lock().unwrap();
        let fill_price = order.price.unwrap_or(state.last_price);
        let fill = OrderFill {
            order_id: id.clone(),
            side: order.side,
            price: fill_price,
            qty: order.qty,
            fee: order.qty * fill_price * 0.001,
            fee_asset: "USDT".into(),
            time: Utc::now(),
        };

        // Update balances
        let cost = order.qty * fill_price;
        if let Some(usdt) = state.balances.iter_mut().find(|b| b.asset == "USDT") {
            usdt.wallet -= cost + fill.fee;
            usdt.cross_wallet = usdt.wallet;
        }
        if order.side == Side::Buy {
            if let Some(btc) = state.balances.iter_mut().find(|b| b.asset == "BTC") {
                btc.wallet += order.qty;
                btc.cross_wallet = btc.wallet;
            }
        } else {
            if let Some(btc) = state.balances.iter_mut().find(|b| b.asset == "BTC") {
                btc.wallet = (btc.wallet - order.qty).max(0.0);
                btc.cross_wallet = btc.wallet;
            }
        }

        if let Some(tx) = state.stream_tx.as_ref() {
            tracing::debug!(target: "mock", "StreamMsg::OrderUpdate id={} price={} qty={}", fill.order_id, fill.price, fill.qty);
            let _ = tx.try_send(StreamMsg::OrderUpdate(fill.clone()));
        }

        if !order.reduce_only {
            let pos_side = match order.side {
                Side::Buy => PositionSide::Long,
                Side::Sell => PositionSide::Short,
            };
            let existing = state
                .positions
                .iter_mut()
                .find(|p| p.symbol == order.symbol.as_ref());
            if let Some(p) = existing {
                if p.side == PositionSide::None || p.side == pos_side {
                    let old_qty = p.size;
                    p.size += order.qty;
                    p.entry_price = (p.entry_price * old_qty + fill_price * order.qty) / p.size;
                    p.side = pos_side;
                } else {
                    let new_size = p.size - order.qty;
                    if new_size > 0.0 {
                        p.size = new_size;
                    } else if new_size < 0.0 {
                        p.size = -new_size;
                        p.side = pos_side;
                        p.entry_price = fill_price;
                    } else {
                        p.size = 0.0;
                        p.side = PositionSide::None;
                        p.entry_price = 0.0;
                    }
                }
            } else {
                state.positions.push(Position {
                    symbol: order.symbol.to_string(),
                    side: pos_side,
                    size: order.qty,
                    entry_price: fill_price,
                    unrealized_pnl: 0.0,
                });
            }
            state.positions.retain(|p| p.size > 0.0);
        } else {
            if let Some(p) = state
                .positions
                .iter_mut()
                .find(|p| p.symbol == order.symbol.as_ref())
            {
                let close_qty = order.qty.min(p.size);
                p.size = (p.size - close_qty).max(0.0);
            }
            state.positions.retain(|p| p.size > 0.0);
        }

        if let Some(tx) = state.stream_tx.as_ref() {
            let info = AccountInfo {
                balances: state.balances.clone(),
                positions: state.positions.clone(),
            };
            tracing::debug!(target: "mock", "StreamMsg::AccountUpdate {} balances, {} positions", info.balances.len(), info.positions.len());
            let _ = tx.try_send(StreamMsg::AccountUpdate(info));
        }

        Ok(id)
    }

    async fn cancel_order(&self, _symbol: &str, _order_id: &str) -> Result<()> {
        Ok(())
    }

    async fn order_status(&self, symbol: &str, _order_id: &str) -> Result<OrderStatus> {
        Ok(OrderStatus {
            order_id: "mock_0".into(),
            symbol: symbol.into(),
            side: Side::Buy,
            qty: 0.0,
            filled_qty: 0.0,
            price: 0.0,
            avg_price: 0.0,
            status: "NEW".into(),
        })
    }

    async fn account_info(&self) -> Result<AccountInfo> {
        let state = self.state.lock().unwrap();
        Ok(AccountInfo {
            balances: state.balances.clone(),
            positions: state.positions.clone(),
        })
    }

    async fn current_price(&self, _symbol: &str) -> Result<f64> {
        let state = self.state.lock().unwrap();
        Ok(state.last_price)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn new_creates_simulated_exchange() {
        let ex = MockExchange::new();
        assert!(ex.public.is_none());
    }

    #[test]
    fn new_public_creates_public_exchange() {
        let ex = MockExchange::new_public();
        assert!(ex.public.is_some());
    }

    #[tokio::test]
    async fn subscribe_generates_trades() {
        let ex = MockExchange::new();
        let stream = ex.subscribe(&["btcusdt".into()]).await.unwrap();
        let msg = tokio::task::spawn_blocking(move || stream.rx.recv())
            .await
            .expect("spawn_blocking panicked")
            .expect("stream closed");
        assert!(matches!(msg, StreamMsg::Trade(_)));
    }

    #[tokio::test]
    async fn subscribe_generates_depth() {
        let ex = MockExchange::new();
        let stream = ex.subscribe(&["btcusdt".into()]).await.unwrap();

        let found_depth = tokio::task::spawn_blocking(move || {
            let mut count = 0;
            loop {
                match stream.rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(StreamMsg::Depth(_)) => return true,
                    Ok(_) => {
                        count += 1;
                        if count > 200 {
                            return false;
                        }
                        continue;
                    }
                    Err(_) => return false,
                }
            }
        })
        .await
        .expect("spawn_blocking panicked");
        assert!(found_depth, "should receive at least one Depth message");
    }

    #[tokio::test]
    async fn place_order_returns_mock_id() {
        let ex = MockExchange::new();
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
        let id = ex.place_order(order).await.unwrap();
        assert!(id.starts_with("mock_"));

        let order2 = Order {
            symbol: "ETHUSDT".into(),
            side: Side::Sell,
            qty: 2.0,
            price: None,
            order_type: OrderType::Market,
            reduce_only: true,
            stop_loss: None,
            take_profit: None,
        };
        let id2 = ex.place_order(order2).await.unwrap();
        assert_ne!(id, id2);
    }

    #[tokio::test]
    async fn cancel_order_returns_ok() {
        let ex = MockExchange::new();
        assert!(ex.cancel_order("BTCUSDT", "1").await.is_ok());
    }

    #[tokio::test]
    async fn order_status_uses_symbol() {
        let ex = MockExchange::new();
        let status = ex.order_status("SOLUSDT", "x").await.unwrap();
        assert_eq!(status.symbol, "SOLUSDT");
        assert_eq!(status.status, "NEW");
        assert_eq!(status.qty, 0.0);
    }

    #[tokio::test]
    async fn account_info_has_usdt_and_btc() {
        let ex = MockExchange::new();
        let info = ex.account_info().await.unwrap();
        assert_eq!(info.balances.len(), 2);
        let usdt = info.balances.iter().find(|b| b.asset == "USDT").unwrap();
        assert_eq!(usdt.wallet, 10000.0);
        let btc = info.balances.iter().find(|b| b.asset == "BTC").unwrap();
        assert!((btc.wallet - 0.1).abs() < 1e-10);
    }

    #[tokio::test]
    async fn current_price_returns_100() {
        let ex = MockExchange::new();
        let price = ex.current_price("ANY").await.unwrap();
        assert!((price - 100.0).abs() < 1e-10);
    }

    #[tokio::test]
    #[ignore = "requires network access to Binance WebSocket"]
    async fn public_subscribe_connects_to_binance() {
        let ex = MockExchange::new_public();
        let stream = ex.subscribe(&["btcusdt".into()]).await.unwrap();
        let msg =
            tokio::task::spawn_blocking(move || stream.rx.recv_timeout(Duration::from_secs(30)))
                .await
                .expect("spawn_blocking panicked")
                .expect("stream closed");
        assert!(matches!(msg, StreamMsg::Trade(_) | StreamMsg::Depth(_)));
    }
}
