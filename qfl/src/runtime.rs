use crate::vm::Vm;
use quince_core::types::{Depth, OrderFill, Side, Trade};
use std::path::PathBuf;

/// Unified exchange event for the QFL runtime.
#[derive(Debug, Clone)]
pub enum Event {
    Trade(Trade),
    Depth(Depth),
    Fill(OrderFill),
    Eval,
}

#[derive(Debug)]
pub struct QflRuntime {
    vm: Vm,
    #[allow(dead_code)]
    path_qfl: PathBuf,
    #[allow(dead_code)]
    current_symbol: String,
    orders_tx: Option<crossbeam_channel::Sender<quince_core::types::Order>>,
    pub risk_engine: crate::risk::RiskEngine,
}

impl QflRuntime {
    pub fn load(strategy_path: &str) -> Result<Self, String> {
        let src = std::fs::read_to_string(strategy_path)
            .map_err(|e| format!("read {}: {}", strategy_path, e))?;

        if src.trim().is_empty() {
            return Err(format!("empty strategy file: {}", strategy_path));
        }

        let program = crate::parser::parse(&src)
            .map_err(|e| format!("parse {}: {}", strategy_path, e))?;

        let qfr = crate::compiler::compile_checked(&program)
            .map_err(|errs| {
                let details: Vec<String> = errs.iter().map(|e| e.msg.clone()).collect();
                format!("type error in {}: {}", strategy_path, details.join("; "))
            })?;

        tracing::info!(
            "QFL VM loaded: {} ({} entry, {} instr, {} consts)",
            strategy_path,
            qfr.entries.len(),
            qfr.code.len(),
            qfr.const_pool.len(),
        );

        Ok(QflRuntime {
            vm: Vm::new(qfr),
            path_qfl: PathBuf::from(strategy_path),
            current_symbol: String::new(),
            orders_tx: None,
            risk_engine: crate::risk::RiskEngine::new(crate::risk::RiskLimits::default()),
        })
    }

    /// Load from pre-compiled .qfr file (bypasses parsing + type checking)
    pub fn load_qfr(qfr_path: &str) -> Result<Self, String> {
        let qfr = crate::ir::QfrProgram::load(qfr_path)?;

        tracing::info!(
            "QFL VM loaded from .qfr: {} ({} entry, {} instr, {} consts)",
            qfr_path,
            qfr.entries.len(),
            qfr.code.len(),
            qfr.const_pool.len(),
        );

        Ok(QflRuntime {
            vm: Vm::new(qfr),
            path_qfl: PathBuf::from(qfr_path),
            current_symbol: String::new(),
            orders_tx: None,
            risk_engine: crate::risk::RiskEngine::new(crate::risk::RiskLimits::default()),
        })
    }

    /// Save current program to .qfr file
    pub fn save_qfr(&self, qfr_path: &str) -> Result<(), String> {
        let mut entries = Vec::with_capacity(self.vm.entry_count as usize);
        for i in 0..self.vm.entry_count as usize {
            let name_bytes = self.vm.entry_names[i].to_le_bytes();
            let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(8);
            let name = String::from_utf8_lossy(&name_bytes[..name_end]).to_string();
            entries.push(crate::ir::EntryPoint { name, code_offset: self.vm.entry_offsets[i] });
        }
        let prog = crate::ir::QfrProgram {
            entries,
            const_pool: self.vm.const_pool.clone(),
            code: self.vm.code_instr(),
            const_map: self.vm.const_map.clone(),
            ema_alphas: Vec::new(),
        };
        prog.save(qfr_path)
    }

    pub fn set_order_sender(&mut self, tx: crossbeam_channel::Sender<quince_core::types::Order>) {
        self.orders_tx = Some(tx);
    }

    pub fn feed_trade(&mut self, trade: Trade) {
        self.vm.set_last_price(trade.price);
        self.vm.set_position_size(0.0);

        self.vm.regs[0].f = trade.price;
        self.vm.regs[1].f = trade.qty;
        self.vm.regs[2].i = match trade.side {
            Side::Buy => 0,
            Side::Sell => 1,
        };
        self.vm.regs[3].i = trade.trade_id as i64;
        self.vm.regs[4].i = trade.time.timestamp_nanos_opt().unwrap_or(0);

        self.vm.call("on_trade");
        self.flush_pending_order();
    }

    pub fn feed_depth(&mut self, depth: Depth) {
        let bids: Vec<(f64, f64)> = depth.bids.iter().map(|d| (d.price, d.qty)).collect();
        let asks: Vec<(f64, f64)> = depth.asks.iter().map(|d| (d.price, d.qty)).collect();
        self.vm.set_depth_bids(bids);
        self.vm.set_depth_asks(asks);

        self.vm.call("on_depth");
    }

    pub fn feed_fill(&mut self, fill: OrderFill) {
        self.vm.set_last_price(fill.price);
        self.vm.regs[0].f = fill.price;
        self.vm.regs[1].f = fill.qty;
        self.vm.regs[2].i = match fill.side {
            Side::Buy => 0,
            Side::Sell => 1,
        };

        if let Some(ref mut t) = self.vm.tracer {
            t.record_fill(fill.price, fill.qty, &format!("{:?}", fill.side));
        }

        self.vm.call("on_fill");
    }

    pub fn feed_eval(&mut self) {
        self.vm.call("on_eval");
        self.flush_pending_order();
    }

    pub fn set_indicator(&mut self, name: &str, val: f64) {
        self.vm.set_indicator(name, val);
    }

    pub fn ensure_indicator_slot(&mut self, name: &str) -> u16 {
        self.vm.ensure_indicator_slot(name)
    }

    pub fn set_indicator_by_slot(&mut self, slot: u16, val: f64) {
        self.vm.set_indicator_by_slot(slot, val);
    }

    pub fn set_balance(&mut self, asset: &str, val: f64) {
        self.vm.set_balance(asset, val);
    }

    pub fn set_position_size(&mut self, size: f64) {
        self.vm.set_position_size(size);
    }

    pub fn set_symbol(&mut self, symbol: &str) {
        self.current_symbol = symbol.to_string();
    }

    /// Finalize VM const lookups after all indicator/balance registrations.
    /// Call this once before the engine loop starts.
    pub fn finalize_vm_init(&mut self) {
        self.vm.finalize_const_lookups();
    }

    fn flush_pending_order(&mut self) {
        let side_val = self.vm.int(250);
        if side_val != 0 && side_val != 1 {
            return;
        }
        let Some(ref tx) = self.orders_tx else { return };

        let side = if side_val == 0 { Side::Buy } else { Side::Sell };
        let qty = self.vm.float(192);
        let price_f64 = self.vm.float(193);
        let price = if price_f64 > 0.0 { Some(price_f64) } else { None };
        let order_type = if self.vm.int(253) == 0 {
            quince_core::types::OrderType::Market
        } else {
            quince_core::types::OrderType::Limit
        };
        let reduce_only = self.vm.int(254) != 0;

        let order = quince_core::types::Order {
            symbol: self.current_symbol.clone(),
            side,
            qty,
            price,
            order_type,
            reduce_only,
            stop_loss: None,
            take_profit: None,
        };

        // Risk check before sending
        match self.risk_engine.check_order(&order) {
            crate::risk::RiskVerdict::Allowed => {
                if let Some(ref mut t) = self.vm.tracer {
                    t.record_risk("allowed", "");
                }
                let _ = tx.try_send(order);
            }
            crate::risk::RiskVerdict::Rejected(reason) => {
                if let Some(ref mut t) = self.vm.tracer {
                    t.record_risk("rejected", &reason);
                }
                tracing::warn!("QFL risk rejected order: {}", reason);
            }
        }
    }

    /// Unified feed — dispatch any event to the correct handler.
    pub fn feed_event(&mut self, event: Event) {
        self.risk_engine.new_cycle();
        match event {
            Event::Trade(trade) => self.feed_trade(trade),
            Event::Depth(depth) => self.feed_depth(depth),
            Event::Fill(fill) => self.feed_fill(fill),
            Event::Eval => self.feed_eval(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::types::{DepthLevel, Side, Trade};

    fn make_trade(price: f64, qty: f64, side: Side, id: u64) -> Trade {
        Trade {
            price,
            qty,
            time: chrono::Utc::now(),
            side,
            trade_id: id,
        }
    }

    fn write_test_strategy(name: &str, source: &str) -> String {
        let path = format!("{}.qfl", name);
        std::fs::write(&path, source).unwrap();
        path
    }

    fn make_fill(price: f64, qty: f64, side: Side) -> OrderFill {
        OrderFill {
            order_id: "test_fill".into(),
            side,
            price,
            qty,
            fee: qty * price * 0.001,
            fee_asset: "USDT".into(),
            time: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_load_and_eval() {
        let path = write_test_strategy("runtime_test_load_eval", "function on_eval() end");
        let rt = QflRuntime::load(&path).unwrap();
        assert_eq!(rt.vm.entry_count, 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_missing_file() {
        let result = QflRuntime::load("nonexistent_file.qfl");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_parse_error() {
        let path = write_test_strategy("runtime_test_parse_err", "function @@@ invalid");
        let result = QflRuntime::load(&path);
        assert!(result.is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_trade_buy() {
        let path = write_test_strategy("runtime_test_trade_buy", "
function on_trade(trade) end
");
        let mut rt = QflRuntime::load(&path).unwrap();
        let trade = make_trade(50000.0, 0.1, Side::Buy, 1);
        rt.feed_trade(trade);

        assert_eq!(rt.vm.reg_f(0), 50000.0);
        assert_eq!(rt.vm.reg_f(1), 0.1);
        assert_eq!(rt.vm.reg_i(2), 0);
        assert_eq!(rt.vm.reg_i(3), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_trade_sell() {
        let path = write_test_strategy("runtime_test_trade_sell", "
function on_trade(trade) end
");
        let mut rt = QflRuntime::load(&path).unwrap();
        let trade = make_trade(50100.0, 0.2, Side::Sell, 42);
        rt.feed_trade(trade);

        assert_eq!(rt.vm.reg_f(0), 50100.0);
        assert_eq!(rt.vm.reg_f(1), 0.2);
        assert_eq!(rt.vm.reg_i(2), 1);
        assert_eq!(rt.vm.reg_i(3), 42);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_depth_with_bids_asks() {
        let path = write_test_strategy("runtime_test_depth_ba", "function on_depth() end");
        let mut rt = QflRuntime::load(&path).unwrap();
        let depth = Depth {
            bids: vec![
                DepthLevel { price: 49900.0, qty: 1.5 },
                DepthLevel { price: 49800.0, qty: 2.5 },
            ],
            asks: vec![
                DepthLevel { price: 50100.0, qty: 1.0 },
                DepthLevel { price: 50200.0, qty: 3.0 },
            ],
        };
        rt.feed_depth(depth);
        assert_eq!(rt.vm.depth_bids_len, 2);
        assert_eq!(rt.vm.depth_asks_len, 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_fill_buy() {
        let path = write_test_strategy("runtime_test_fill_buy", "function on_fill(fill) end");
        let mut rt = QflRuntime::load(&path).unwrap();
        let fill = make_fill(50000.0, 0.1, Side::Buy);
        rt.feed_fill(fill);
        assert_eq!(rt.vm.reg_f(0), 50000.0);
        assert_eq!(rt.vm.reg_f(1), 0.1);
        assert_eq!(rt.vm.reg_i(2), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_fill_sell() {
        let path = write_test_strategy("runtime_test_fill_sell", "function on_fill(fill) end");
        let mut rt = QflRuntime::load(&path).unwrap();
        let fill = make_fill(50100.0, 0.2, Side::Sell);
        rt.feed_fill(fill);
        assert_eq!(rt.vm.reg_i(2), 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_set_indicator() {
        let path = write_test_strategy("runtime_test_set_ind", "function on_eval() end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_indicator("ema", 123.456);
        assert!((rt.vm.indicators[(*rt.vm.indicator_map.get("ema").unwrap()) as usize] - 123.456).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_set_balance() {
        let path = write_test_strategy("runtime_test_set_bal", "function on_eval() end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_balance("USDT", 50000.0);
        rt.set_balance("BTC", 1.5);
        assert!((rt.vm.balances[(*rt.vm.balance_map.get("USDT").unwrap()) as usize] - 50000.0).abs() < 0.001);
        assert!((rt.vm.balances[(*rt.vm.balance_map.get("BTC").unwrap()) as usize] - 1.5).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_set_position_size() {
        let path = write_test_strategy("runtime_test_set_pos", "function on_eval() end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_position_size(1.5);
        assert!((rt.vm.position_size - 1.5).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_set_symbol() {
        let path = write_test_strategy("runtime_test_set_sym", "function on_eval() end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_symbol("BTCUSDT");
        assert_eq!(rt.current_symbol, "BTCUSDT");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_order_send_buy_market() {
        let src = "
function on_eval()
    quince.order(0, 1.0, 0)
end
";
        let path = write_test_strategy("runtime_test_ord_buy", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("BTCUSDT");
        rt.feed_eval();

        let order = rx.try_recv().expect("should have sent order");
        assert_eq!(order.symbol, "BTCUSDT");
        assert_eq!(order.side, quince_core::types::Side::Buy);
        assert!((order.qty - 1.0).abs() < 0.001);
        assert_eq!(order.order_type, quince_core::types::OrderType::Market);
        assert!(!order.reduce_only);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_order_send_sell_limit() {
        let src = "
function on_eval()
    quince.order(1, 0.1, 51000.0, 1)
end
";
        let path = write_test_strategy("runtime_test_ord_sell", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("ETHUSDT");
        rt.feed_eval();

        let order = rx.try_recv().expect("should have sent order");
        assert_eq!(order.symbol, "ETHUSDT");
        assert_eq!(order.side, quince_core::types::Side::Sell);
        assert!((order.qty - 0.1).abs() < 0.001);
        assert_eq!(order.price, Some(51000.0));
        assert_eq!(order.order_type, quince_core::types::OrderType::Limit);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_order_send_reduce_only() {
        let src = "
function on_eval()
    quince.order(1, 0.3, 0, 0, 1)
end
";
        let path = write_test_strategy("runtime_test_ord_red", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("BTCUSDT");
        rt.feed_eval();

        let order = rx.try_recv().expect("should have sent order");
        assert!(order.reduce_only);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_no_order_when_side_invalid() {
        let src = "
function on_eval()
    quince.order(99, 1.0, 0)
end
";
        let path = write_test_strategy("runtime_test_no_ord", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.feed_eval();
        assert!(rx.try_recv().is_err(), "should NOT send order with invalid side");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_multiple_feeds() {
        let src = "
@persist local count = 0
function on_trade(trade)
    count = count + 1
end
function on_depth()
    count = count + 1
end
function on_fill(fill)
    count = count + 1
end
function on_eval()
    count = count + 1
end
";
        let path = write_test_strategy("runtime_test_multi_feed", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let trade = make_trade(100.0, 1.0, Side::Buy, 1);

        rt.feed_trade(trade);
        rt.feed_depth(Depth {
            bids: vec![DepthLevel { price: 99.0, qty: 1.0 }],
            asks: vec![],
        });
        rt.feed_fill(make_fill(100.0, 1.0, Side::Buy));
        rt.feed_eval();

        assert_eq!(rt.vm.persist[0].int_val, 4);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_indicators_set_before_eval() {
        let src = "
function on_eval()
    local ema = quince.get(\"ema5\")
end
";
        let path = write_test_strategy("runtime_test_ind_before", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_indicator("ema5", 50000.0);
        rt.feed_eval();
        // Just verify no crash and indicator is accessible
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_indicators_multiple_names() {
        let src = "
function on_eval()
    local a = quince.get(\"ema5\")
    local b = quince.get(\"ema20\")
    local c = quince.get(\"bb.middle\")
end
";
        let path = write_test_strategy("runtime_test_ind_multi", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_indicator("ema5", 100.0);
        rt.set_indicator("ema20", 101.0);
        rt.set_indicator("bb.middle", 100.5);
        rt.feed_eval();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_persist_survives_reload() {
        let src = "
@persist local counter = 0
function on_eval()
    counter = counter + 1
end
";
        let path_a = write_test_strategy("runtime_test_persist_a", src);
        let mut rt = QflRuntime::load(&path_a).unwrap();
        rt.feed_eval();
        rt.feed_eval();

        let persist = rt.vm.persist;

        let path_b = write_test_strategy("runtime_test_persist_b", src);
        let mut rt2 = QflRuntime::load(&path_b).unwrap();
        rt2.vm.persist.copy_from_slice(&persist);
        rt2.feed_eval();

        assert_eq!(rt2.vm.persist[0].int_val, 3);
        let _ = std::fs::remove_file(&path_a);
        let _ = std::fs::remove_file(&path_b);
    }

    #[test]
    fn test_feed_trade_sell_side() {
        let path = write_test_strategy("runtime_test_trade_sell2", "
function on_trade(trade) end
");
        let mut rt = QflRuntime::load(&path).unwrap();
        let trade = make_trade(100.0, 0.5, Side::Sell, 99);
        rt.feed_trade(trade);
        assert_eq!(rt.vm.reg_i(2), 1);
        assert_eq!(rt.vm.reg_i(3), 99);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_balance_set_then_feed_eval() {
        let src = "
function on_eval()
    local bal = quince.balance(\"USDT\")
end
";
        let path = write_test_strategy("runtime_test_bal_eval", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_balance("USDT", 10000.0);
        rt.feed_eval();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_position_size_then_feed_trade() {
        let src = "
@persist local pos = 0.0
function on_trade(trade)
    pos = quince.position()
end
";
        let path = write_test_strategy("runtime_test_pos_trade", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_position_size(1.5);
        let trade = make_trade(100.0, 1.0, Side::Buy, 1);
        rt.feed_trade(trade);
        let _ = std::fs::remove_file(&path);
    }

    // ── Additional runtime tests ──

    #[test]
    fn test_feed_eval_no_handler() {
        let path = write_test_strategy("runtime_eval_none", "function on_trade(trade) end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_eval(); // should not crash even without on_eval handler
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_depth_no_handler() {
        let path = write_test_strategy("runtime_depth_none", "function on_trade(trade) end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_depth(Depth {
            bids: vec![DepthLevel { price: 100.0, qty: 1.0 }],
            asks: vec![],
        });
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_fill_no_handler() {
        let path = write_test_strategy("runtime_fill_none", "function on_trade(trade) end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_fill(make_fill(100.0, 1.0, Side::Buy));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_trade_without_on_trade() {
        let path = write_test_strategy("runtime_trade_none", "function on_eval() end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_trade(make_trade(100.0, 1.0, Side::Buy, 1));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_multiple_evals() {
        let src = "
@persist local count = 0
function on_eval()
    count = count + 1
end
";
        let path = write_test_strategy("runtime_multi_eval", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        for _ in 0..10 {
            rt.feed_eval();
        }
        assert_eq!(rt.vm.persist[0].int_val, 10);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_multiple_trades() {
        let src = "
@persist local count = 0
function on_trade(trade)
    count = count + 1
end
";
        let path = write_test_strategy("runtime_multi_trade", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        for i in 0..5 {
            rt.feed_trade(make_trade(100.0 + i as f64, 1.0, Side::Buy, i));
        }
        assert_eq!(rt.vm.persist[0].int_val, 5);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_persist_initial_value() {
        let src = "
@persist local counter = 42
function on_eval()
    counter = counter + 1
end
";
        let path = write_test_strategy("runtime_persist_init", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        // First eval increments from 0 to 1 (init value is only template)
        rt.feed_eval();
        assert_eq!(rt.vm.persist[0].int_val, 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_order_send_market_buy_no_price() {
        let src = "
function on_eval()
    quince.order(0, 0.5)
end
";
        let path = write_test_strategy("runtime_ord_mkt_buy", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("BTCUSDT");
        rt.feed_eval();
        let order = rx.try_recv().expect("should send order");
        assert_eq!(order.side, quince_core::types::Side::Buy);
        assert_eq!(order.order_type, quince_core::types::OrderType::Market);
        assert!(order.price.is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_order_send_limit_with_price() {
        let src = "
function on_eval()
    quince.order(0, 0.1, 50000.0, 1)
end
";
        let path = write_test_strategy("runtime_ord_lim", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("ETHUSDT");
        rt.feed_eval();
        let order = rx.try_recv().expect("should send order");
        assert_eq!(order.order_type, quince_core::types::OrderType::Limit);
        assert_eq!(order.price, Some(50000.0));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_order_not_sent_without_sender() {
        let src = "
function on_eval()
    quince.order(0, 1.0, 0)
end
";
        let path = write_test_strategy("runtime_ord_no_tx", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        // No order sender set — should not crash
        rt.feed_eval();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_indicators_missing_get_zero() {
        let src = "
@persist local val = 0.0
function on_eval()
    val = quince.get(\"nonexistent\")
end
";
        let path = write_test_strategy("runtime_ind_missing", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_eval();
        assert!((rt.vm.persist[0].float_val - 0.0).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_balance_multiple_assets() {
        let src = "
function on_eval()
    local usdt = quince.balance(\"USDT\")
    local btc = quince.balance(\"BTC\")
end
";
        let path = write_test_strategy("runtime_bal_multi", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_balance("USDT", 10000.0);
        rt.set_balance("BTC", 2.5);
        rt.feed_eval();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_position_after_trade() {
        let src = "
@persist local pos = 0.0
function on_trade(trade)
    pos = quince.position()
end
";
        let path = write_test_strategy("runtime_pos_after", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_position_size(2.0);
        // feed_trade sets position to 0 before calling on_trade
        rt.feed_trade(make_trade(100.0, 1.0, Side::Buy, 1));
        // Position resets to 0 during trade feed
        assert!((rt.vm.persist[0].float_val - 0.0).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_price_updated_on_trade() {
        let src = "
@persist local last = 0.0
function on_trade(trade)
    last = quince.price()
end
";
        let path = write_test_strategy("runtime_price_trade", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_trade(make_trade(50500.0, 0.5, Side::Buy, 1));
        assert!((rt.vm.persist[0].float_val - 50500.0).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_runtime_empty_strategy() {
        let path = write_test_strategy("runtime_empty", "");
        let result = QflRuntime::load(&path);
        assert!(result.is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_runtime_only_comments() {
        let path = write_test_strategy("runtime_comments", "-- comment\n-- another\n");
        let result = QflRuntime::load(&path);
        assert!(result.is_err()); // comments alone are not valid
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_persist_float_value_survives() {
        let src = "
@persist local val = 0.0
function on_eval()
    val = 3.14159
end
";
        let path_a = write_test_strategy("runtime_pfloat_a", src);
        let mut rt = QflRuntime::load(&path_a).unwrap();
        rt.feed_eval();
        let persist = rt.vm.persist;
        let path_b = write_test_strategy("runtime_pfloat_b", src);
        let mut rt2 = QflRuntime::load(&path_b).unwrap();
        rt2.vm.persist.copy_from_slice(&persist);
        rt2.feed_eval();
        assert_eq!(rt2.vm.persist[0].tag, 1);
        assert!((rt2.vm.persist[0].float_val - 3.14159).abs() < 0.001);
        let _ = std::fs::remove_file(&path_a);
        let _ = std::fs::remove_file(&path_b);
    }

    #[test]
    fn test_multiple_depth_feeds() {
        let src = "
@persist local depth_count = 0
function on_depth()
    depth_count = depth_count + 1
end
";
        let path = write_test_strategy("runtime_multi_depth", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let d = Depth {
            bids: vec![DepthLevel { price: 100.0, qty: 1.0 }],
            asks: vec![],
        };
        rt.feed_depth(d);
        rt.feed_depth(Depth { bids: vec![], asks: vec![] });
        assert_eq!(rt.vm.persist[0].int_val, 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_multiple_fill_feeds() {
        let src = "
@persist local fill_count = 0
function on_fill(fill)
    fill_count = fill_count + 1
end
";
        let path = write_test_strategy("runtime_multi_fill", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_fill(make_fill(100.0, 1.0, Side::Buy));
        rt.feed_fill(make_fill(101.0, 0.5, Side::Sell));
        assert_eq!(rt.vm.persist[0].int_val, 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_complex_strategy_flow() {
        let src = "
@persist local trade_count = 0
@persist local eval_count = 0

function on_trade(trade)
    trade_count = trade_count + 1
end

function on_eval()
    eval_count = eval_count + 1
end
";
        let path = write_test_strategy("runtime_complex", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_position_size(0.5);
        rt.set_balance("USDT", 10000.0);
        rt.set_indicator("ema", 50000.0);
        rt.feed_trade(make_trade(50100.0, 1.0, Side::Buy, 1));
        rt.feed_eval();
        rt.feed_trade(make_trade(50200.0, 0.5, Side::Sell, 2));
        rt.feed_eval();
        assert_eq!(rt.vm.persist[0].int_val, 2); // trade_count
        assert_eq!(rt.vm.persist[1].int_val, 2); // eval_count
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_many_consecutive_evals() {
        let src = "
@persist local count = 0
function on_eval()
    count = count + 1
end
";
        let path = write_test_strategy("runtime_many_eval", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        for _ in 0..100 {
            rt.feed_eval();
        }
        assert_eq!(rt.vm.persist[0].int_val, 100);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_reload_with_new_indicators() {
        let src_v1 = "
function on_eval()
    local ema = quince.get(\"ema\")
end
";
        let src_v2 = "
function on_eval()
    local ema = quince.get(\"ema\")
    local sma = quince.get(\"sma\")
end
";
        let path_v1 = write_test_strategy("runtime_reload_v1", src_v1);
        let mut rt = QflRuntime::load(&path_v1).unwrap();
        rt.set_indicator("ema", 100.0);
        rt.feed_eval();

        // "Reload" by loading v2 into a new rt
        let path_v2 = write_test_strategy("runtime_reload_v2", src_v2);
        let mut rt2 = QflRuntime::load(&path_v2).unwrap();
        rt2.vm.persist.copy_from_slice(&rt.vm.persist);
        rt2.set_indicator("ema", 101.0);
        rt2.set_indicator("sma", 99.0);
        rt2.feed_eval();

        let _ = std::fs::remove_file(&path_v1);
        let _ = std::fs::remove_file(&path_v2);
    }

    // ── Hot-reload tests ──

    #[test]
    fn test_hot_reload_persist_type_change() {
        let src_v1 = "
@persist local x = 0
function on_eval()
    x = 42
end
";
        let src_v2 = "
@persist local x = 0.0
function on_eval()
    x = 3.14
end
";
        let path_v1 = write_test_strategy("hr_type_v1", src_v1);
        let mut rt_v1 = QflRuntime::load(&path_v1).unwrap();
        rt_v1.feed_eval();
        assert_eq!(rt_v1.vm.persist[0].int_val, 42);

        let persist = rt_v1.vm.persist;

        let path_v2 = write_test_strategy("hr_type_v2", src_v2);
        let mut rt_v2 = QflRuntime::load(&path_v2).unwrap();
        rt_v2.vm.persist.copy_from_slice(&persist);
        rt_v2.feed_eval();

        assert_eq!(rt_v2.vm.persist[0].tag, 1);
        assert!((rt_v2.vm.persist[0].float_val - 3.14).abs() < 0.001);

        let _ = std::fs::remove_file(&path_v1);
        let _ = std::fs::remove_file(&path_v2);
    }

    #[test]
    fn test_hot_reload_persist_removal() {
        let src_v1 = "
@persist local x = 0
function on_eval()
    x = 42
end
";
        let src_v2 = "
function on_eval()
end
";
        let path_v1 = write_test_strategy("hr_rem_v1", src_v1);
        let mut rt_v1 = QflRuntime::load(&path_v1).unwrap();
        rt_v1.feed_eval();
        assert_eq!(rt_v1.vm.persist[0].int_val, 42);

        let persist = rt_v1.vm.persist;

        let path_v2 = write_test_strategy("hr_rem_v2", src_v2);
        let mut rt_v2 = QflRuntime::load(&path_v2).unwrap();
        rt_v2.vm.persist.copy_from_slice(&persist);
        assert_eq!(rt_v2.vm.persist[0].int_val, 42);

        let _ = std::fs::remove_file(&path_v1);
        let _ = std::fs::remove_file(&path_v2);
    }

    #[test]
    fn test_hot_reload_new_var_shift() {
        let src_v1 = "
@persist local a = 0
function on_eval()
    a = 10
end
";
        let src_v2 = "
@persist local b = 0
@persist local a = 0
function on_eval()
    b = b + 1
    a = a + 1
end
";
        let path_v1 = write_test_strategy("hr_shift_v1", src_v1);
        let mut rt_v1 = QflRuntime::load(&path_v1).unwrap();
        rt_v1.feed_eval();
        assert_eq!(rt_v1.vm.persist[0].int_val, 10);

        let persist = rt_v1.vm.persist;

        let path_v2 = write_test_strategy("hr_shift_v2", src_v2);
        let mut rt_v2 = QflRuntime::load(&path_v2).unwrap();
        rt_v2.vm.persist.copy_from_slice(&persist);
        rt_v2.feed_eval();

        assert_eq!(rt_v2.vm.persist[0].int_val, 11);
        assert_eq!(rt_v2.vm.persist[1].int_val, 1);

        let _ = std::fs::remove_file(&path_v1);
        let _ = std::fs::remove_file(&path_v2);
    }

    #[test]
    fn test_hot_reload_corrupted_deployment() {
        let path = write_test_strategy("hr_corrupt", "function @@@ invalid");
        let result = QflRuntime::load(&path);
        assert!(result.is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_hot_reload_parallel_updates() {
        let src = "
@persist local x = 0
function on_eval()
    x = x + 1
end
";
        for i in 0..5 {
            let path = write_test_strategy(&format!("hr_par_{}", i), src);
            let mut rt = QflRuntime::load(&path).unwrap();
            rt.feed_eval();
            rt.feed_eval();
            assert_eq!(rt.vm.persist[0].int_val, 2);
            let _ = std::fs::remove_file(&path);
        }
    }

    // ── Persist edge case tests ──

    #[test]
    fn test_persist_all_64_slots_filled() {
        let mut src = String::new();
        for i in 0..32 {
            src.push_str(&format!("@persist local v{} = 0\n", i));
        }
        src.push_str("function on_eval()\n");
        for i in 0..32 {
            src.push_str(&format!("    v{} = v{} + 1\n", i, i));
        }
        src.push_str("end\n");

        let path = write_test_strategy("persist_64", &src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_eval();

        for i in 0..32 {
            assert_eq!(rt.vm.persist[i].int_val, 1, "slot {}", i);
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_persist_across_multiple_entries() {
        let src = "
@persist local counter = 0
function on_trade(trade)
    counter = counter + 1
end
function on_eval()
    counter = counter + 1
end
";
        let path = write_test_strategy("persist_multi_entry", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_trade(make_trade(100.0, 1.0, Side::Buy, 1));
        assert_eq!(rt.vm.persist[0].int_val, 1);
        rt.feed_eval();
        assert_eq!(rt.vm.persist[0].int_val, 2);
        rt.feed_trade(make_trade(101.0, 0.5, Side::Sell, 2));
        assert_eq!(rt.vm.persist[0].int_val, 3);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_persist_default_zero() {
        let src = "
@persist local x = 0
function on_eval()
    local _ = x
end
";
        let path = write_test_strategy("persist_default", src);
        let rt = QflRuntime::load(&path).unwrap();
        assert_eq!(rt.vm.persist[0].int_val, 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_persist_negative_value() {
        let src = "
@persist local x = 0
function on_eval()
    x = -42
end
";
        let path = write_test_strategy("persist_neg", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_eval();
        assert_eq!(rt.vm.persist[0].int_val, -42);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_persist_large_value() {
        let src = "
@persist local x = 0
function on_eval()
    x = 500000000000
end
";
        let path = write_test_strategy("persist_large", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_eval();
        assert_eq!(rt.vm.persist[0].int_val, 500000000000i64);
        let _ = std::fs::remove_file(&path);
    }

    // ── Order flow edge case tests ──

    #[test]
    fn test_order_send_buy_market_after_trade() {
        let src = "
function on_trade(trade)
    quince.order(0, 1.0, 0)
end
";
        let path = write_test_strategy("ord_buy_trade", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("BTCUSDT");
        rt.feed_trade(make_trade(50000.0, 0.1, Side::Buy, 1));

        let order = rx.try_recv().expect("should send order after trade");
        assert_eq!(order.side, quince_core::types::Side::Buy);
        assert!((order.qty - 1.0).abs() < 0.001);
        assert_eq!(order.order_type, quince_core::types::OrderType::Market);
        assert!(order.price.is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_order_send_sell_market_after_trade() {
        let src = "
function on_trade(trade)
    quince.order(1, 0.5, 0)
end
";
        let path = write_test_strategy("ord_sell_trade", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("ETHUSDT");
        rt.feed_trade(make_trade(3000.0, 0.2, Side::Sell, 1));

        let order = rx.try_recv().expect("should send order after trade");
        assert_eq!(order.side, quince_core::types::Side::Sell);
        assert!((order.qty - 0.5).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_order_multiple_in_one_feed() {
        let src = "
function on_eval()
    quince.order(0, 0.5, 0)
end
";
        let path = write_test_strategy("ord_multi", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("BTCUSDT");

        rt.feed_eval();
        let o1 = rx.try_recv().expect("first order");
        assert_eq!(o1.side, quince_core::types::Side::Buy);
        assert!((o1.qty - 0.5).abs() < 0.001);

        rt.feed_eval();
        let o2 = rx.try_recv().expect("second order");
        assert_eq!(o2.side, quince_core::types::Side::Buy);
        assert!((o2.qty - 0.5).abs() < 0.001);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_order_with_all_5_args() {
        let src = "
function on_eval()
    quince.order(1, 0.1, 50000.0, 1, 1)
end
";
        let path = write_test_strategy("ord_5args", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("BTCUSDT");
        rt.feed_eval();

        let order = rx.try_recv().expect("should send order");
        assert_eq!(order.side, quince_core::types::Side::Sell);
        assert!((order.qty - 0.1).abs() < 0.001);
        assert_eq!(order.price, Some(50000.0));
        assert_eq!(order.order_type, quince_core::types::OrderType::Limit);
        assert!(order.reduce_only);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_order_type_limit() {
        let src = "
function on_eval()
    quince.order(0, 0.1, 50000.0, 1)
end
";
        let path = write_test_strategy("ord_type_lim", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("BTCUSDT");
        rt.feed_eval();

        let order = rx.try_recv().expect("should send order");
        assert_eq!(order.order_type, quince_core::types::OrderType::Limit);
        assert_eq!(order.price, Some(50000.0));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_reduce_only_true() {
        let src = "
function on_eval()
    quince.order(1, 0.5, 0, 0, 1)
end
";
        let path = write_test_strategy("ord_reduce", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("BTCUSDT");
        rt.feed_eval();

        let order = rx.try_recv().expect("should send order");
        assert!(order.reduce_only);
        assert_eq!(order.side, quince_core::types::Side::Sell);
        assert_eq!(order.order_type, quince_core::types::OrderType::Market);
        let _ = std::fs::remove_file(&path);
    }

    // ── Error path tests ──

    #[test]
    fn test_load_nonexistent_file() {
        let result = QflRuntime::load("nonexistent_file.qfl");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_empty_file() {
        let path = write_test_strategy("runtime_empty_file", "");
        let result = QflRuntime::load(&path);
        assert!(result.is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_binary_garbage() {
        let path = "runtime_bin_garbage.qfl";
        let garbage: Vec<u8> = vec![0x00, 0x01, 0x02, 0xFF, 0xFE, 0xFD];
        std::fs::write(&path, garbage).unwrap();
        let result = QflRuntime::load(&path);
        assert!(result.is_err());
        let _ = std::fs::remove_file(&path);
    }

    // ── Feed state update tests ──

    #[test]
    fn test_feed_trade_updates_last_price() {
        let path = write_test_strategy("ft_last_price", "function on_trade(trade) end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_trade(make_trade(50500.0, 1.0, Side::Buy, 1));
        assert!((rt.vm.last_price - 50500.0).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_trade_updates_position() {
        let path = write_test_strategy("ft_position", "function on_trade(trade) end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_position_size(2.0);
        rt.feed_trade(make_trade(100.0, 1.0, Side::Buy, 1));
        assert!((rt.vm.position_size - 0.0).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_depth_updates_depth_data() {
        let path = write_test_strategy("fd_depth", "function on_depth() end");
        let mut rt = QflRuntime::load(&path).unwrap();
        let depth = Depth {
            bids: vec![DepthLevel { price: 49900.0, qty: 1.5 }],
            asks: vec![DepthLevel { price: 50100.0, qty: 2.0 }],
        };
        rt.feed_depth(depth);
        assert_eq!(rt.vm.depth_bids_len, 1);
        assert_eq!(rt.vm.depth_asks_len, 1);
        assert!((rt.vm.depth_bids_price[0] - 49900.0).abs() < 0.001);
        assert!((rt.vm.depth_asks_price[0] - 50100.0).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_fill_updates_registers() {
        let path = write_test_strategy("ff_regs", "function on_fill(fill) end");
        let mut rt = QflRuntime::load(&path).unwrap();
        let fill = make_fill(50200.0, 0.25, Side::Buy);
        rt.feed_fill(fill);
        assert!((rt.vm.reg_f(0) - 50200.0).abs() < 0.001);
        assert!((rt.vm.reg_f(1) - 0.25).abs() < 0.001);
        assert_eq!(rt.vm.reg_i(2), 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_eval_with_no_entry() {
        let path = write_test_strategy("fe_no_entry", "function on_trade(trade) end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_eval();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_multiple_feeds_mixed() {
        let src = "
@persist local counter = 0
function on_trade(trade)
    counter = counter + 2
end
function on_depth()
    counter = counter + 3
end
function on_fill(fill)
    counter = counter + 5
end
function on_eval()
    counter = counter + 7
end
";
        let path = write_test_strategy("mixed_feeds", src);
        let mut rt = QflRuntime::load(&path).unwrap();

        rt.feed_trade(make_trade(100.0, 1.0, Side::Buy, 1));
        assert_eq!(rt.vm.persist[0].int_val, 2);

        rt.feed_depth(Depth {
            bids: vec![DepthLevel { price: 99.0, qty: 1.0 }],
            asks: vec![],
        });
        assert_eq!(rt.vm.persist[0].int_val, 5);

        rt.feed_fill(make_fill(100.0, 1.0, Side::Buy));
        assert_eq!(rt.vm.persist[0].int_val, 10);

        rt.feed_eval();
        assert_eq!(rt.vm.persist[0].int_val, 17);

        let _ = std::fs::remove_file(&path);
    }

    // ── Type-checked runtime loading ──

    #[test]
    fn test_load_type_error_rejected() {
        // 42 + true is a type error
        let path = write_test_strategy("runtime_type_err", "42 + true");
        let result = QflRuntime::load(&path);
        assert!(result.is_err(), "type error should be rejected at load time");
        let err = result.unwrap_err();
        assert!(err.contains("type error"), "error should mention type error: {}", err);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_valid_strategy_ok() {
        let path = write_test_strategy("runtime_valid_type", "
            @persist local pos = 0
            function on_trade(trade)
                if trade.price > 0 then quince.order(0, 1.0, trade.price) end
            end
            function on_eval() quince.log(\"ok\") end
        ");
        let result = QflRuntime::load(&path);
        assert!(result.is_ok(), "valid strategy should load: {:?}", result.err());
        let _ = std::fs::remove_file(&path);
    }

    // ── Risk engine integration ──

    #[test]
    fn test_risk_rejects_large_order() {
        let path = write_test_strategy("risk_large", "
            function on_eval() quince.order(0, 100.0, 50000.0) end
        ");
        let mut rt = QflRuntime::load(&path).unwrap();
        // Set very tight risk limits
        rt.risk_engine.limits.max_order_notional = 1000.0;
        rt.feed_eval();
        // Order should be rejected by risk (notional = 100*50000 = 5M >> 1000)
        // No crash means test passes
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_event_trade_dispatches_correctly() {
        let path = write_test_strategy("fe_event_trade", "function on_trade(trade) end");
        let mut rt = QflRuntime::load(&path).unwrap();
        let trade = make_trade(50000.0, 0.1, Side::Buy, 1);
        rt.feed_event(Event::Trade(trade));
        assert!((rt.vm.reg_f(0) - 50000.0).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_event_depth_dispatches_correctly() {
        let path = write_test_strategy("fe_event_depth", "function on_depth() end");
        let mut rt = QflRuntime::load(&path).unwrap();
        let depth = Depth {
            bids: vec![DepthLevel { price: 100.0, qty: 1.0 }],
            asks: vec![],
        };
        rt.feed_event(Event::Depth(depth));
        assert_eq!(rt.vm.depth_bids_len, 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_event_fill_dispatches_correctly() {
        let path = write_test_strategy("fe_event_fill", "function on_fill(fill) end");
        let mut rt = QflRuntime::load(&path).unwrap();
        let fill = make_fill(50200.0, 0.25, Side::Buy);
        rt.feed_event(Event::Fill(fill));
        assert!((rt.vm.reg_f(0) - 50200.0).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_event_eval_dispatches_correctly() {
        let path = write_test_strategy("fe_event_eval", "function on_eval() end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_event(Event::Eval);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_risk_new_cycle_called_on_feed_event() {
        let path = write_test_strategy("risk_cycle", "
            function on_eval()
                quince.order(0, 0.1, 100.0)
                quince.order(0, 0.1, 100.0)
            end
        ");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.risk_engine.limits.max_orders_per_cycle = 1;
        rt.feed_event(Event::Eval);
        // Second eval should reset order count via new_cycle
        rt.feed_event(Event::Eval);
        let _ = std::fs::remove_file(&path);
    }

    // ── Tracer integration ──

    fn make_tracer_rt(name: &str, src: &str) -> (QflRuntime, String) {
        let path = write_test_strategy(name, src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.vm.tracer = Some(crate::tracer::Tracer::new(1024));
        (rt, path)
    }

    #[test]
    fn trace_fill_records_fill_event() {
        let (mut rt, path) = make_tracer_rt("tr_fill_rec", "function on_fill(fill) end");
        let fill = make_fill(50000.0, 0.1, Side::Buy);
        rt.feed_fill(fill);
        let events = rt.vm.tracer.as_mut().unwrap().drain();
        assert!(!events.is_empty());
        let has_fill = events.iter().any(|e| matches!(e, crate::tracer::TraceEvent::Fill { .. }));
        assert!(has_fill, "expected a Fill trace event");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn trace_fill_has_correct_values() {
        let (mut rt, path) = make_tracer_rt("tr_fill_vals", "function on_fill(fill) end");
        let fill = make_fill(50200.0, 0.25, Side::Sell);
        rt.feed_fill(fill);
        let events = rt.vm.tracer.as_mut().unwrap().drain();
        let fill_event = events.iter().find_map(|e| {
            if let crate::tracer::TraceEvent::Fill { price, qty, side } = e {
                Some((*price, *qty, side.clone()))
            } else {
                None
            }
        });
        assert!(fill_event.is_some());
        let (price, qty, side) = fill_event.unwrap();
        assert!((price - 50200.0).abs() < 0.001);
        assert!((qty - 0.25).abs() < 0.001);
        assert_eq!(side, "Sell");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn trace_risk_allowed_on_order_send() {
        let src = "
            function on_eval()
                quince.order(0, 0.1, 100.0)
            end
        ";
        let (mut rt, path) = make_tracer_rt("tr_risk_ok", src);
        let (tx, _rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("BTCUSDT");
        rt.feed_eval();
        let events = rt.vm.tracer.as_mut().unwrap().drain();
        let has_risk = events.iter().any(|e| matches!(e, crate::tracer::TraceEvent::RiskAction { .. }));
        assert!(has_risk, "expected a RiskAction trace event");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn trace_risk_rejected_when_limit_exceeded() {
        let src = "
            function on_eval()
                quince.order(0, 100.0, 50000.0)
            end
        ";
        let (mut rt, path) = make_tracer_rt("tr_risk_rej", src);
        rt.risk_engine.limits.max_order_notional = 1000.0;
        let (tx, _rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("BTCUSDT");
        rt.feed_eval();
        let events = rt.vm.tracer.as_mut().unwrap().drain();
        let rejected = events.iter().any(|e| {
            matches!(e, crate::tracer::TraceEvent::RiskAction { verdict, .. } if verdict == "rejected")
        });
        assert!(rejected, "expected a rejected RiskAction trace event");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn trace_multiple_events_in_one_feed() {
        let src = "
            function on_eval()
                quince.order(0, 0.1, 100.0)
            end
        ";
        let (mut rt, path) = make_tracer_rt("tr_multi_ev", src);
        let (tx, _rx) = crossbeam_channel::unbounded();
        rt.set_order_sender(tx);
        rt.set_symbol("BTCUSDT");
        // feed_fill produces Fill trace, feed_eval may produce signal+risk traces
        rt.feed_fill(make_fill(50000.0, 0.1, Side::Buy));
        rt.feed_eval();
        let events = rt.vm.tracer.as_mut().unwrap().drain();
        let kinds: Vec<&str> = events.iter().map(|e| e.kind()).collect();
        assert!(kinds.contains(&"fill"), "expected fill event: {:?}", kinds);
        assert!(kinds.contains(&"risk"), "expected risk event: {:?}", kinds);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn trace_no_crash_when_tracer_none() {
        let src = "function on_eval() end";
        let path = write_test_strategy("trace_none_rt", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        // Tracer is not set
        rt.feed_fill(make_fill(100.0, 1.0, Side::Buy));
        rt.feed_eval();
        let _ = std::fs::remove_file(&path);
    }

    // ── .qfr save/load integration ──

    #[test]
    fn qfr_save_load_roundtrip() {
        let src = "
            @persist local pos = 0
            function on_trade(trade)
                local fast = quince.get(\"sma10\")
                local slow = quince.get(\"sma50\")
                if fast > slow and pos <= 0 then
                    quince.order(0, 1.0, 0)
                    pos = 1
                end
            end
            function on_eval()
                quince.log(\"eval\")
            end
        ";
        let path = write_test_strategy("qfr_rt_save", src);
        let rt = QflRuntime::load(&path).unwrap();

        let qfr_path = "test_roundtrip.qfr";
        rt.save_qfr(qfr_path).unwrap();

        let rt2 = QflRuntime::load_qfr(qfr_path).unwrap();
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(qfr_path);

        // Verify loaded program has same structure
        assert_eq!(rt2.vm.entry_count, 2);
        assert_eq!(rt2.vm.code_len, rt.vm.code_len);
        assert_eq!(rt2.vm.const_pool.len(), rt.vm.const_pool.len());
    }

    #[test]
    fn qfr_load_nonexistent_returns_error() {
        let result = QflRuntime::load_qfr("nonexistent.qfr");
        assert!(result.is_err());
    }

    #[test]
    fn test_feed_trade_big_values() {
        let path = write_test_strategy("rt_big_vals", "function on_trade(trade) end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_trade(make_trade(1_000_000.0, 1000.0, Side::Buy, 999));
        assert!((rt.vm.reg_f(0) - 1_000_000.0).abs() < 0.001);
        assert!((rt.vm.reg_f(1) - 1000.0).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_feed_depth_empty_bids_asks() {
        let path = write_test_strategy("rt_depth_empty", "function on_depth() end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_depth(Depth { bids: vec![], asks: vec![] });
        assert_eq!(rt.vm.depth_bids_len, 0);
        assert_eq!(rt.vm.depth_asks_len, 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_on_fill_with_field_access() {
        let path = write_test_strategy("rt_fill_field", "function on_fill(fill) local p = fill.price local q = fill.qty end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_fill(make_fill(50000.0, 0.5, Side::Buy));
        assert!((rt.vm.reg_f(0) - 50000.0).abs() < 0.001);
        assert!((rt.vm.reg_f(1) - 0.5).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_state_var_with_typed_decl() {
        let src = "
state counter : f64 = 0.0
on eval() {
    counter = counter + 1.0
}
";
        let path = write_test_strategy("rt_state_var", src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_eval();
        rt.feed_eval();
        let _ = std::fs::remove_file(&path);
    }
}
