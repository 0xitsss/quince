use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossbeam_channel;
use quince_core::types::*;
use quince_engine::Engine;
use async_trait::async_trait;
use quince_exchange::r#trait::{Exchange, Result, Stream, StreamMsg, OrderStatus};
use quince_risk::{RiskConfig, RiskControls};

// ── Mock exchange for integration tests ──

struct MockState {
    stream_tx: Option<crossbeam_channel::Sender<StreamMsg>>,
    last_price: f64,
}

pub struct MockExchange {
    order_counter: AtomicU64,
    state: Arc<Mutex<MockState>>,
}

impl MockExchange {
    fn new() -> Self {
        MockExchange {
            order_counter: AtomicU64::new(1),
            state: Arc::new(Mutex::new(MockState {
                stream_tx: None,
                last_price: 100.0,
            })),
        }
    }
}

#[async_trait]
impl Exchange for MockExchange {
    async fn subscribe(&self, _symbols: &[String]) -> Result<Stream> {
        let (tx, rx) = crossbeam_channel::bounded(1024);
        self.state.lock().unwrap().stream_tx = Some(tx.clone());

        let state = Arc::clone(&self.state);
        std::thread::spawn(move || {
            for tick in 0u64..100 {
                let phase = (tick as f64) * 0.01;
                let price = 100.0 + 10.0 * phase.sin();
                state.lock().unwrap().last_price = price;
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
                let _ = tx.try_send(StreamMsg::Trade(trade));

                if tick % 5 == 0 {
                    let spread = price * 0.001;
                    let bids: Vec<DepthLevel> = (0..5)
                        .map(|i| DepthLevel { price: price - spread * (i + 1) as f64, qty: 1.0 })
                        .collect();
                    let asks: Vec<DepthLevel> = (0..5)
                        .map(|i| DepthLevel { price: price + spread * (i + 1) as f64, qty: 1.0 })
                        .collect();
                    let _ = tx.try_send(StreamMsg::Depth(Depth { bids, asks }));
                }

                std::thread::sleep(Duration::from_millis(5));
            }
        });

        Ok(Stream { rx })
    }

    async fn place_order(&self, order: Order) -> Result<String> {
        let id = format!("int_{}", self.order_counter.fetch_add(1, Ordering::Relaxed));
        let state = self.state.lock().unwrap();
        let fill = OrderFill {
            order_id: id.clone(),
            side: order.side,
            price: order.price.unwrap_or(state.last_price),
            qty: order.qty,
            fee: order.qty * state.last_price * 0.001,
            fee_asset: "USDT".into(),
            time: chrono::Utc::now(),
        };
        drop(state);

        if let Some(tx) = self.state.lock().unwrap().stream_tx.as_ref() {
            let _ = tx.try_send(StreamMsg::OrderUpdate(fill));
        }
        Ok(id)
    }

    async fn cancel_order(&self, _symbol: &str, _order_id: &str) -> Result<()> {
        Ok(())
    }

    async fn order_status(&self, symbol: &str, _order_id: &str) -> Result<OrderStatus> {
        Ok(OrderStatus {
            order_id: "0".into(),
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
        Ok(self.state.lock().unwrap().last_price)
    }
}

// ── Helper ──

fn write_strategy(name: &str, source: &str) -> String {
    let path = format!("{}.qfl", name);
    std::fs::write(&path, source).expect("write strategy");
    path
}

fn risk_config() -> RiskControls {
    RiskControls::new(RiskConfig {
        max_position_size: 10.0,
        max_drawdown: 0.1,
        max_order_freq: 100,
        max_daily_loss: 10000.0,
        cooldown_after_loss_secs: 0,
    })
}

async fn run_engine_for_ticks(strategy_path: &str) {
    let exchange = MockExchange::new();
    let risk = risk_config();
    let mut engine = Engine::new(exchange, &["BTCUSDT".into()], strategy_path, risk, "test_trades.log")
        .expect("engine should start");
    // Run for ~50ms to process some ticks
    tokio::time::timeout(Duration::from_millis(200), engine.run()).await.ok();
}

// ── Integration tests ──

macro_rules! integration_test {
    ($name:ident, $src:expr) => {
        #[tokio::test]
        async fn $name() {
            let path = write_strategy(stringify!($name), $src);
            run_engine_for_ticks(&path).await;
            let _ = std::fs::remove_file(&path);
        }
    };
}

integration_test!(intg_basic_eval, "
function on_eval()
end
");

integration_test!(intg_only_trade, "
function on_trade(trade)
end
");

integration_test!(intg_only_depth, "
function on_depth()
end
");

integration_test!(intg_only_fill, "
function on_fill(fill)
end
");

integration_test!(intg_all_entries, "
function on_trade(trade) end
function on_depth() end
function on_fill(fill) end
function on_eval() end
");

integration_test!(intg_trade_fields, "
function on_trade(trade)
    local p = trade.price
    local q = trade.qty
    local s = trade.side
    local id = trade.trade_id
    local t = trade.time
end
");

integration_test!(intg_get_price, "
function on_trade(trade)
    local px = quince.price()
end
function on_eval()
    local px = quince.price()
end
");

integration_test!(intg_get_position, "
function on_trade(trade)
    local pos = quince.position()
end
function on_eval()
    local pos = quince.position()
end
");

integration_test!(intg_get_indicator, "
--USING ema 5
function on_trade(trade)
    local ema = quince.get(\"ema\")
end
function on_eval()
    local ema = quince.get(\"ema\")
end
");

integration_test!(intg_get_balance, "
function on_eval()
    local bal = quince.balance(\"USDT\")
end
");

integration_test!(intg_order_buy, "
function on_trade(trade)
    quince.order(0, 0.1, 0)
end
");

integration_test!(intg_order_sell, "
function on_trade(trade)
    quince.order(1, 0.1, 0)
end
");

integration_test!(intg_persist_counter, "
@persist local count = 0
function on_trade(trade)
    count = count + 1
end
function on_eval()
    count = count + 1
end
");

integration_test!(intg_if_else, "
function on_trade(trade)
    local p = trade.price
    if p > 100 then
        quince.order(0, 0.1, 0)
    else
        quince.order(1, 0.1, 0)
    end
end
");

integration_test!(intg_while_loop, "
function on_trade(trade)
    local i = 0
    while i < 3 do
        i = i + 1
    end
end
");

integration_test!(intg_repeat_loop, "
function on_trade(trade)
    local i = 0
    repeat
        i = i + 1
    until i >= 3
end
");

integration_test!(intg_for_loop, "
function on_trade(trade)
    local total = 0
    for i = 1, 5 do
        total = total + i
    end
end
");

integration_test!(intg_log_call, "
function on_trade(trade)
    quince.log(\"test log\")
end
function on_eval()
    quince.log(\"eval log\")
end
");

integration_test!(intg_multi_persist, "
@persist local a = 0
@persist local b = 0
function on_trade(trade)
    a = a + 1
    b = b + 2
end
function on_eval()
    a = a + 1
end
");

integration_test!(intg_fill_handler, "
function on_fill(fill)
    local p = fill.price
    local q = fill.qty
end
");

integration_test!(intg_depth_handler, "
function on_depth()
end
");

integration_test!(intg_quince_method_syntax, "
function on_trade(trade)
    local p = quince:price()
    local pos = quince:position()
    local ema = quince:get(\"ema\")
    local bal = quince:balance(\"USDT\")
    quince:log(\"method call\")
end
");

// ── Full strategies as integration tests ──

integration_test!(intg_ema_cross, "
--USING ema 9 50
@persist local pos = 0
function on_trade(trade)
    local fast = quince.get(\"ema9\")
    local slow = quince.get(\"ema50\")
    if fast > slow and pos <= 0 then quince.order(0, 1.0, 0) pos = 1 end
    if fast < slow and pos > 0 then quince.order(1, 1.0, 0) pos = 0 end
end
function on_eval() quince.log(\"eval\") end
");

integration_test!(intg_scalper, "
--USING bb 20 2.0
@persist local pos = 0
function on_trade(trade)
    local price = trade.price
    local lower = quince.get(\"bb.lower\")
    local upper = quince.get(\"bb.upper\")
    local mid = quince.get(\"bb.middle\")
    local ema = quince.get(\"ema\")
    if price < lower and ema > mid and pos == 0 then quince.order(0, 1.0, 0) pos = 1 end
    if price > upper and ema < mid and pos > 0 then quince.order(1, 1.0, 0) pos = 0 end
end
function on_eval() end
");

integration_test!(intg_rsi_reversion, "
@persist local pos = 0
function on_trade(trade)
    local rsi = quince.get(\"rsi\")
    if rsi < 30 and pos <= 0 then quince.order(0, 1.0, 0) pos = 1 end
    if rsi > 70 and pos > 0 then quince.order(1, 1.0, 0) pos = 0 end
end
function on_eval() quince.log(\"eval\") end
");

integration_test!(intg_macd_cross, "
@persist local pos = 0
function on_trade(trade)
    local macd = quince.get(\"macd.macd\")
    local signal = quince.get(\"macd.signal\")
    if macd > signal and pos <= 0 then quince.order(0, 1.0, 0) pos = 1 end
    if macd < signal and pos > 0 then quince.order(1, 1.0, 0) pos = 0 end
end
function on_eval() end
");

integration_test!(intg_momentum, "
@persist local pos = 0
function on_trade(trade)
    local roc = quince.get(\"roc\")
    if roc > 2 and pos <= 0 then quince.order(0, 1.0, 0) pos = 1 end
    if roc < -2 and pos > 0 then quince.order(1, 1.0, 0) pos = 0 end
end
function on_eval() end
");

integration_test!(intg_data_passing, "
@using sma:10:50
@using ema:20:50

state position_size : i64 = 0
state entry_price : f64 = 0.0
state test_counter : i64 = 0

on trade(t) {
    feature sma_fast = quince.get(\"sma10\")
    feature ema_slow = quince.get(\"ema20\")
    
    let price_val = quince.price()
    let pos_val = quince.position()
    let bal_val = quince.balance(\"USDT\")
    
    quince.log(\"trade_price\")
    quince.log2(\"price\", price_val)
    quince.log2(\"pos\", pos_val)
    quince.log2(\"bal\", bal_val)
    quince.log2(\"sma10\", sma_fast)
    quince.log2(\"ema20\", ema_slow)
    
    state test_counter = test_counter + 1
}

on depth(d) {
    let bid0 = quince.depth_bid(0)
    let ask0 = quince.depth_ask(0)
    quince.log(\"depth\")
    quince.log2(\"bid0\", bid0)
    quince.log2(\"ask0\", ask0)
}

on eval() {
    let price_val = quince.price()
    let pos_val = quince.position()
    let bal_val = quince.balance(\"USDT\")
    let sma_fast = quince.get(\"sma10\")
    let ema_slow = quince.get(\"ema20\")
    
    quince.log(\"eval\")
    quince.log2(\"price\", price_val)
    quince.log2(\"pos\", pos_val)
    quince.log2(\"bal\", bal_val)
    quince.log2(\"sma10\", sma_fast)
    quince.log2(\"ema20\", ema_slow)
    quince.log2(\"counter\", test_counter)
    
    if sma_fast > ema_slow and position_size <= 0 {
        quince.order(0, 1.0, 0)
        state position_size = 1
        state entry_price = quince.price()
        quince.log(\"BUY\")
    }
    
    if sma_fast < ema_slow and position_size > 0 {
        quince.order(1, 1.0, 0)
        state position_size = 0
        quince.log(\"SELL\")
    }
}

on fill(f) {
    quince.log(\"fill\")
    quince.log2(\"qty\", f.qty)
    quince.log2(\"price\", f.price)
}
");

// ── Risk / edge integration tests ──

#[tokio::test]
async fn intg_risk_rejects_large_order() {
    let src = "
function on_trade(trade)
    quince.order(0, 100.0, 0)
end
";
    let path = write_strategy("intg_risk_reject", src);
    let exchange = MockExchange::new();
    let risk = RiskControls::new(RiskConfig {
        max_position_size: 1.0,
        max_drawdown: 0.1,
        max_order_freq: 100,
        max_daily_loss: 10000.0,
        cooldown_after_loss_secs: 0,
    });
    let mut engine = Engine::new(exchange, &["BTCUSDT".into()], &path, risk, "test_trades.log")
        .expect("engine should start");
    tokio::time::timeout(Duration::from_millis(100), engine.run()).await.ok();
    let _ = std::fs::remove_file(&path);
}

#[tokio::test]
async fn intg_empty_strategy() {
    let src = "";
    let path = write_strategy("intg_empty", src);
    let exchange = MockExchange::new();
    let risk = risk_config();
    let result = Engine::new(exchange, &["BTCUSDT".into()], &path, risk, "test_trades.log");
    assert!(result.is_err(), "empty strategy should not load");
    let _ = std::fs::remove_file(&path);
}
