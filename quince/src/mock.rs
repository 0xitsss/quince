use quince::exchange::r#trait::{Exchange, OrderStatus, Result, Stream, StreamMsg};
use quince::core::types::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct MockExchange {
    order_counter: AtomicU64,
}

impl MockExchange {
    pub fn new() -> Self {
        Self { order_counter: AtomicU64::new(1) }
    }
}

#[async_trait::async_trait]
impl Exchange for MockExchange {
    async fn subscribe(&self, _symbols: &[String]) -> Result<Stream> {
        let (tx, rx) = tokio::sync::mpsc::channel(1024);

        tokio::spawn(async move {
            let mut tick = 0u64;
            let mut depth_tick = 0u64;
            loop {
                let phase = (tick as f64) * 0.01;
                let price = 100.0 + 10.0 * phase.sin();
                let qty = 0.1 + 0.05 * (phase * 3.0).sin();
                let side = if (phase * 2.0).sin() > 0.0 { Side::Buy } else { Side::Sell };
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
                let _ = tx.send(StreamMsg::Trade(trade)).await;

                if depth_tick % 5 == 0 {
                    let spread = price * 0.001;
                    let bids: Vec<DepthLevel> = (0..10)
                        .map(|i| DepthLevel { price: price - spread * (i + 1) as f64, qty: 1.0 - i as f64 * 0.1 })
                        .collect();
                    let asks: Vec<DepthLevel> = (0..10)
                        .map(|i| DepthLevel { price: price + spread * (i + 1) as f64, qty: 1.0 - i as f64 * 0.1 })
                        .collect();
                    let _ = tx.send(StreamMsg::Depth(Depth { bids, asks })).await;
                }

                tick += 1;
                depth_tick += 1;
                tokio::time::sleep(Duration::from_millis(10)).await;

                if tick > 5000 { break; }
            }
        });

        Ok(Stream { rx })
    }

    async fn place_order(&self, _order: Order) -> Result<String> {
        let id = self.order_counter.fetch_add(1, Ordering::Relaxed);
        Ok(format!("mock_{}", id))
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
        Ok(AccountInfo {
            balances: vec![
                Balance { asset: "USDT".into(), wallet: 10000.0, cross_wallet: 10000.0 },
                Balance { asset: "BTC".into(), wallet: 0.1, cross_wallet: 0.1 },
            ],
            positions: vec![],
        })
    }

    async fn current_price(&self, _symbol: &str) -> Result<f64> {
        Ok(100.0)
    }
}
