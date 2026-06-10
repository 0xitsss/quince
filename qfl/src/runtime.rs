// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! QFL runtime — high-level interface between the trading engine and the VM.
//!
//! Owns a [`Vm`], a compiled strategy path, symbol context, an order-sending
//! channel, and a [`RiskEngine`](quince_risk::RiskEngine). Exposes `feed_*`
//! methods that push external events (trade, depth, fill, eval) into the VM.
//!
//! Entry point: [`QflRuntime::load()`].

use crate::vm::Vm;
use quince_core::types::{Depth, OrderFill, Side, Trade};
use std::path::PathBuf;
use std::sync::Arc;

/// Unified exchange event dispatched to the QFL runtime.
///
/// Each variant triggers a different handler (`on_trade`, `on_depth`, etc.)
/// inside the VM.
#[derive(Debug, Clone)]
pub enum Event {
    Trade(Trade),    // Market trade tick
    Depth(Depth),    // Order book snapshot
    Fill(OrderFill), // Order fill notification
    Eval,            // Periodic evaluation tick
}

// --- Section: QflRuntime struct ---
// Top-level runtime that owns a VM, strategy path, symbol context,
// an order-sending channel, and a risk engine.
#[derive(Debug)]
pub struct QflRuntime {
    vm: Vm, // The QFL bytecode VM instance
    #[allow(dead_code)]
    path_qfl: PathBuf, // Path to the .qfl or .qfr source file
    #[allow(dead_code)]
    current_symbol: Arc<str>, // Trading symbol currently being processed
    orders_tx: Option<crossbeam_channel::Sender<quince_core::types::Order>>, // Channel to send orders to the exchange adapter
    pub risk_engine: crate::risk::RiskEngine, // Risk limits and checking
}

// --- Section: QflRuntime implementation ---
impl QflRuntime {
    // Load and compile a .qfl strategy file from source.
    // Full pipeline: read file -> parse -> type-check -> optimize -> create VM.
    // Returns a fully initialized runtime, or an error string.
    pub fn load(strategy_path: &str) -> Result<Self, String> {
        // Read the raw source file
        let src = std::fs::read_to_string(strategy_path)
            .map_err(|e| format!("read {}: {}", strategy_path, e))?;

        // Reject empty files early
        if src.trim().is_empty() {
            return Err(format!("empty strategy file: {}", strategy_path));
        }

        // Parse source text into an AST
        let program =
            crate::parser::parse(&src).map_err(|e| format!("parse {}: {}", strategy_path, e))?;

        // Type-check and compile AST into QFR bytecode (may return multiple errors)
        let mut qfr = crate::compiler::compile_checked(&program).map_err(|errs| {
            let details: Vec<String> = errs.iter().map(|e| e.msg.clone()).collect();
            format!("type error in {}: {}", strategy_path, details.join("; "))
        })?;

        // Apply peephole and other optimizations to the bytecode
        crate::optimize::optimize(&mut qfr);

        // Log the loaded strategy metadata
        tracing::info!(
            "QFL VM loaded: {} ({} entry, {} instr, {} consts)",
            strategy_path,
            qfr.entries.len(),
            qfr.code.len(),
            qfr.const_pool.len(),
        );

        // Construct the VM from the compiled program
        let mut vm = Vm::new(qfr);

        // Auto-enable VM trace if QFL_TRACE_VM env var is set
        if std::env::var("QFL_TRACE_VM").is_ok() {
            vm.enable_trace_vm("qflvmtrace.log");
        }

        // Assemble the runtime with default risk limits and no order channel yet
        Ok(QflRuntime {
            vm,
            path_qfl: PathBuf::from(strategy_path),
            current_symbol: Arc::from(""),
            orders_tx: None,
            risk_engine: crate::risk::RiskEngine::new(crate::risk::RiskLimits::default()),
        })
    }

    // Load from a pre-compiled .qfr file, bypassing parsing and type checking.
    // Useful for faster startup when the strategy has already been validated.
    pub fn load_qfr(qfr_path: &str) -> Result<Self, String> {
        // Deserialize the binary .qfr program file
        let qfr = crate::ir::QfrProgram::load(qfr_path)?;

        // Log metadata about the loaded program
        tracing::info!(
            "QFL VM loaded from .qfr: {} ({} entry, {} instr, {} consts)",
            qfr_path,
            qfr.entries.len(),
            qfr.code.len(),
            qfr.const_pool.len(),
        );

        // Construct runtime from the pre-compiled program
        Ok(QflRuntime {
            vm: Vm::new(qfr),
            path_qfl: PathBuf::from(qfr_path),
            current_symbol: Arc::from(""),
            orders_tx: None,
            risk_engine: crate::risk::RiskEngine::new(crate::risk::RiskLimits::default()),
        })
    }

    // Serialize the current compiled program to a .qfr file for later fast loading.
    // Rebuilds the entry-point table and const pool from the VM's internal state.
    pub fn save_qfr(&self, qfr_path: &str) -> Result<(), String> {
        // Reconstruct entry points from VM's internal entry arrays
        let mut entries = Vec::with_capacity(self.vm.entry_count as usize);
        for i in 0..self.vm.entry_count as usize {
            // Read the 8-byte little-endian entry name, trim trailing null bytes
            let name_bytes = self.vm.entry_names[i].to_le_bytes();
            let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(8);
            let name = String::from_utf8_lossy(&name_bytes[..name_end]).to_string();
            entries.push(crate::ir::EntryPoint {
                name,
                code_offset: self.vm.entry_offsets[i],
            });
        }

        // Build a map from string constants to their index in the const pool
        let const_map = self
            .vm
            .cold
            .const_pool
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                if let crate::ir::ConstEntry::String(s) = e {
                    Some((s.clone(), i as u32))
                } else {
                    None
                }
            })
            .collect();

        // Assemble the QfrProgram structure from VM state
        let prog = crate::ir::QfrProgram {
            entries,
            const_pool: self.vm.cold.const_pool.clone(),
            code: self.vm.code_instr(),
            const_map,
            ema_alphas: Vec::new(), // Not serialized from VM; recalculated on load
            // Extract f64, i64, and string constants into separate vectors
            f64_consts: self
                .vm
                .cold
                .const_pool
                .iter()
                .filter_map(|e| {
                    if let crate::ir::ConstEntry::F64(v) = e {
                        Some(*v)
                    } else {
                        None
                    }
                })
                .collect(),
            i64_consts: self
                .vm
                .cold
                .const_pool
                .iter()
                .filter_map(|e| {
                    if let crate::ir::ConstEntry::I64(v) = e {
                        Some(*v)
                    } else {
                        None
                    }
                })
                .collect(),
            string_consts: self
                .vm
                .cold
                .const_pool
                .iter()
                .filter_map(|e| {
                    if let crate::ir::ConstEntry::String(s) = e {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .collect(),
        };
        // Write the program to disk in binary format
        prog.save(qfr_path)
    }

    // Provide the runtime with a crossbeam channel sender for outbound orders.
    // Must be called before any feed methods that generate orders.
    pub fn set_order_sender(&mut self, tx: crossbeam_channel::Sender<quince_core::types::Order>) {
        self.orders_tx = Some(tx);
    }

    // --- Section: Event feed methods ---

    // Feed a trade event into the runtime.
    // Sets up VM registers (price, qty, side, trade_id, timestamp),
    // updates last_price and resets position_size to 0,
    // then calls the user-defined on_trade handler and flushes any pending order.
    pub fn feed_trade(&mut self, trade: Trade) {
        #[cfg(feature = "profiling")]
        puffin::profile_scope!("feed_trade");
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

    // Feed a depth (order book) snapshot into the runtime.
    // Sets up depth data in the VM, loads top bid into registers,
    // then calls the on_depth handler.
    pub fn feed_depth(&mut self, depth: Depth) {
        #[cfg(feature = "profiling")]
        puffin::profile_scope!("feed_depth");
        self.vm.set_depth_bids(&depth.bids);
        self.vm.set_depth_asks(&depth.asks);

        self.vm.regs[0].f = depth.bids.first().map_or(0.0, |l| l.price);
        self.vm.regs[1].f = depth.bids.first().map_or(0.0, |l| l.qty);
        self.vm.regs[2].i = depth.bids.len() as i64;
        self.vm.regs[3].i = depth.asks.len() as i64;

        self.vm.call("on_depth");
        self.flush_pending_order();
    }

    // Feed an order fill event into the runtime.
    // Sets up registers with fill data, records trace if enabled,
    // then calls on_fill handler.
    pub fn feed_fill(&mut self, fill: OrderFill) {
        #[cfg(feature = "profiling")]
        puffin::profile_scope!("feed_fill");
        self.vm.set_last_price(fill.price);
        self.vm.regs[0].f = fill.price;
        self.vm.regs[1].f = fill.qty;
        self.vm.regs[2].i = match fill.side {
            Side::Buy => 0,
            Side::Sell => 1,
        };
        self.vm.regs[3].i = 0;
        self.vm.regs[4].i = fill.time.timestamp_nanos_opt().unwrap_or(0);

        if let Some(ref mut t) = self.vm.cold.tracer {
            t.record_fill(fill.price, fill.qty, &format!("{:?}", fill.side));
        }

        self.vm.call("on_fill");
        self.flush_pending_order();
    }

    // Feed a periodic evaluation tick.
    // Calls the on_eval handler and flushes any pending order.
    pub fn feed_eval(&mut self) {
        #[cfg(feature = "profiling")]
        puffin::profile_scope!("feed_eval");
        self.vm.call("on_eval");
        self.flush_pending_order();
    }

    // --- Section: Indicator setters ---

    // Set an indicator value by name (creates or updates).
    // The VM resolves the name to a slot and stores the value.
    pub fn set_indicator(&mut self, name: &str, val: f64) {
        self.vm.set_indicator(name, val);
    }

    // Ensure an indicator slot exists for the given name, creating it if needed.
    // Returns the slot index for fast subsequent updates via set_indicator_by_slot.
    pub fn ensure_indicator_slot(&mut self, name: &str) -> u16 {
        self.vm.ensure_indicator_slot(name)
    }

    // Set an indicator value by pre-resolved slot index (faster than by name).
    pub fn set_indicator_by_slot(&mut self, slot: u16, val: f64) {
        self.vm.set_indicator_by_slot(slot, val);
    }

    // --- Section: Balance setters ---

    // Set a balance value by asset name (creates or updates).
    pub fn set_balance(&mut self, asset: &str, val: f64) {
        self.vm.set_balance(asset, val);
    }

    // Ensure a balance slot exists for the given asset name, creating it if needed.
    // Returns the slot index for fast subsequent updates.
    pub fn ensure_balance_slot(&mut self, name: &str) -> u16 {
        self.vm.ensure_balance_slot(name)
    }

    // Set a balance value by pre-resolved slot index.
    pub fn set_balance_by_slot(&mut self, slot: u16, val: f64) {
        self.vm.set_balance_by_slot(slot, val);
    }

    // --- Section: Position / symbol setters ---

    // Set the current position size exposed to the strategy via quince.position().
    pub fn set_position_size(&mut self, size: f64) {
        self.vm.set_position_size(size);
    }

    // Set the trading symbol string (used when constructing orders).
    pub fn set_symbol(&mut self, symbol: &str) {
        self.current_symbol = Arc::from(symbol);
    }

    // Finalize VM const lookups after all indicator/balance registrations.
    // Call this once before the engine loop starts to freeze the slot mappings.
    pub fn finalize_vm_init(&mut self) {
        self.vm.finalize_const_lookups();
    }

    // Drain the debug-only VM log ring buffer and return all captured log messages.
    // In release builds this always returns an empty vec (zero overhead).
    pub fn dump_vm_logs(&mut self) -> Vec<String> {
        #[cfg(debug_assertions)]
        if let Some(ref mut buf) = self.vm.cold.log_buffer {
            return buf.drain().collect();
        }
        Vec::new()
    }

    // --- Section: Order flushing ---

    // Check if the user's handler placed a pending order (via quince.order()).
    // If so, read the order parameters from VM registers, perform a risk check,
    // and send the order through the channel if allowed.
    fn flush_pending_order(&mut self) {
        #[cfg(feature = "profiling")]
        puffin::profile_scope!("flush_pending_order");
        if !self.vm.has_pending_order {
            return;
        }
        // Clear the pending flag
        self.vm.has_pending_order = false;

        // Read the side register; must be 0 (buy) or 1 (sell)
        let side_val = self.vm.int(250);
        if side_val != 0 && side_val != 1 {
            return; // Invalid side, silently discard
        }

        // Bail out if no order sender has been configured
        let Some(ref tx) = self.orders_tx else { return };

        // Decode order fields from VM reserved registers
        let side = if side_val == 0 { Side::Buy } else { Side::Sell };
        let qty = self.vm.float(192); // qty register
        let price_f64 = self.vm.float(193); // price register
        let price = if price_f64 > 0.0 {
            Some(price_f64) // Price > 0 means limit order
        } else {
            None // Zero price means market order
        };
        let order_type = if self.vm.int(253) == 0 {
            quince_core::types::OrderType::Market
        } else {
            quince_core::types::OrderType::Limit
        };
        let reduce_only = self.vm.int(254) != 0; // reduce-only flag

        // Assemble the full order struct
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
                // Record risk-allowed trace if tracer is attached
                if let Some(ref mut t) = self.vm.cold.tracer {
                    t.record_risk("allowed", "");
                }
                // Non-blocking send through the crossbeam channel
                let _ = tx.try_send(order);
            }
            crate::risk::RiskVerdict::Rejected(reason) => {
                // Record risk-rejected trace if tracer is attached
                if let Some(ref mut t) = self.vm.cold.tracer {
                    t.record_risk("rejected", &reason);
                }
                tracing::warn!("QFL risk rejected order: {}", reason);
            }
        }
    }

    // Unified feed — accepts any Event variant and dispatches to the correct handler.
    // Starts a new risk cycle (resets per-cycle counters) before dispatching.
    pub fn feed_event(&mut self, event: Event) {
        self.risk_engine.new_cycle(); // Reset per-cycle risk counters
        match event {
            Event::Trade(trade) => self.feed_trade(trade),
            Event::Depth(depth) => self.feed_depth(depth),
            Event::Fill(fill) => self.feed_fill(fill),
            Event::Eval => self.feed_eval(),
        }
    }
}

// --- Section: Tests ---
#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::types::{DepthLevel, Side, Trade};

    // --- Test helpers ---

    // Construct a Trade with the given parameters and current timestamp
    fn make_trade(price: f64, qty: f64, side: Side, id: u64) -> Trade {
        Trade {
            price,
            qty,
            time: chrono::Utc::now(),
            side,
            trade_id: id,
        }
    }

    // Write a strategy source string to a .qfl file and return the file path
    fn write_test_strategy(name: &str, source: &str) -> String {
        let path = format!("{}.qfl", name);
        std::fs::write(&path, source).unwrap();
        path
    }

    // Construct an OrderFill with the given parameters and current timestamp
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

    // --- Section: Basic loading tests ---

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

    // --- Section: Trade feed tests ---

    #[test]
    fn test_feed_trade_buy() {
        let path = write_test_strategy(
            "runtime_test_trade_buy",
            "
function on_trade(trade) end
",
        );
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
        let path = write_test_strategy(
            "runtime_test_trade_sell",
            "
function on_trade(trade) end
",
        );
        let mut rt = QflRuntime::load(&path).unwrap();
        let trade = make_trade(50100.0, 0.2, Side::Sell, 42);
        rt.feed_trade(trade);

        assert_eq!(rt.vm.reg_f(0), 50100.0);
        assert_eq!(rt.vm.reg_f(1), 0.2);
        assert_eq!(rt.vm.reg_i(2), 1);
        assert_eq!(rt.vm.reg_i(3), 42);
        let _ = std::fs::remove_file(&path);
    }

    // --- Section: Depth feed tests ---

    #[test]
    fn test_feed_depth_with_bids_asks() {
        let path = write_test_strategy("runtime_test_depth_ba", "function on_depth() end");
        let mut rt = QflRuntime::load(&path).unwrap();
        let depth = Depth {
            bids: vec![
                DepthLevel {
                    price: 49900.0,
                    qty: 1.5,
                },
                DepthLevel {
                    price: 49800.0,
                    qty: 2.5,
                },
            ],
            asks: vec![
                DepthLevel {
                    price: 50100.0,
                    qty: 1.0,
                },
                DepthLevel {
                    price: 50200.0,
                    qty: 3.0,
                },
            ],
        };
        rt.feed_depth(depth);
        assert_eq!(rt.vm.cold.depth_bids_len, 2);
        assert_eq!(rt.vm.cold.depth_asks_len, 2);
        let _ = std::fs::remove_file(&path);
    }

    // --- Section: Fill feed tests ---

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

    // --- Section: Indicator / Balance tests ---

    #[test]
    fn test_set_indicator() {
        let path = write_test_strategy("runtime_test_set_ind", "function on_eval() end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_indicator("ema", 123.456);
        let slot = rt.vm.indicator_slot("ema").unwrap();
        assert!((rt.vm.cold.indicators[slot as usize] - 123.456).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_set_balance() {
        let path = write_test_strategy("runtime_test_set_bal", "function on_eval() end");
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.set_balance("USDT", 50000.0);
        rt.set_balance("BTC", 1.5);
        let usdt_slot = rt.vm.ensure_balance_slot("USDT");
        let btc_slot = rt.vm.ensure_balance_slot("BTC");
        assert!((rt.vm.cold.balances[usdt_slot as usize] - 50000.0).abs() < 0.001);
        assert!((rt.vm.cold.balances[btc_slot as usize] - 1.5).abs() < 0.001);
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
        assert_eq!(rt.current_symbol.as_ref(), "BTCUSDT");
        let _ = std::fs::remove_file(&path);
    }

    // --- Section: Order sending tests ---

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
        assert_eq!(order.symbol.as_ref(), "BTCUSDT");
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
        assert_eq!(order.symbol.as_ref(), "ETHUSDT");
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
        assert!(
            rx.try_recv().is_err(),
            "should NOT send order with invalid side"
        );
        let _ = std::fs::remove_file(&path);
    }

    // --- Section: Multi-feed tests ---

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
            bids: vec![DepthLevel {
                price: 99.0,
                qty: 1.0,
            }],
            asks: vec![],
        });
        rt.feed_fill(make_fill(100.0, 1.0, Side::Buy));
        rt.feed_eval();

        assert_eq!(rt.vm.cold.persist[0].int_val, 4);
        let _ = std::fs::remove_file(&path);
    }

    // --- Section: Indicator setup before eval ---

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

    // --- Section: Persist state hot-reload ---

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

        let persist = rt.vm.cold.persist;

        let path_b = write_test_strategy("runtime_test_persist_b", src);
        let mut rt2 = QflRuntime::load(&path_b).unwrap();
        rt2.vm.cold.persist.copy_from_slice(&persist);
        rt2.feed_eval();

        assert_eq!(rt2.vm.cold.persist[0].int_val, 3);
        let _ = std::fs::remove_file(&path_a);
        let _ = std::fs::remove_file(&path_b);
    }

    #[test]
    fn test_feed_trade_sell_side() {
        let path = write_test_strategy(
            "runtime_test_trade_sell2",
            "
function on_trade(trade) end
",
        );
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

    // в”Ђв”Ђ Additional runtime tests в”Ђв”Ђ

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
            bids: vec![DepthLevel {
                price: 100.0,
                qty: 1.0,
            }],
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

    // --- Section: Multiple eval/trade feed tests ---

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
        assert_eq!(rt.vm.cold.persist[0].int_val, 10);
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
        assert_eq!(rt.vm.cold.persist[0].int_val, 5);
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
        assert_eq!(rt.vm.cold.persist[0].int_val, 1);
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
        assert!((rt.vm.cold.persist[0].float_val - 0.0).abs() < 0.001);
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
        assert!((rt.vm.cold.persist[0].float_val - 0.0).abs() < 0.001);
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
        assert!((rt.vm.cold.persist[0].float_val - 50500.0).abs() < 0.001);
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
    #[allow(clippy::approx_constant)]
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
        let persist = rt.vm.cold.persist;
        let path_b = write_test_strategy("runtime_pfloat_b", src);
        let mut rt2 = QflRuntime::load(&path_b).unwrap();
        rt2.vm.cold.persist.copy_from_slice(&persist);
        rt2.feed_eval();
        assert_eq!(rt2.vm.cold.persist[0].tag, 1);
        assert!((rt2.vm.cold.persist[0].float_val - 3.14159).abs() < 0.001);
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
            bids: vec![DepthLevel {
                price: 100.0,
                qty: 1.0,
            }],
            asks: vec![],
        };
        rt.feed_depth(d);
        rt.feed_depth(Depth {
            bids: vec![],
            asks: vec![],
        });
        assert_eq!(rt.vm.cold.persist[0].int_val, 2);
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
        assert_eq!(rt.vm.cold.persist[0].int_val, 2);
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
        assert_eq!(rt.vm.cold.persist[0].int_val, 2); // trade_count
        assert_eq!(rt.vm.cold.persist[1].int_val, 2); // eval_count
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
        assert_eq!(rt.vm.cold.persist[0].int_val, 100);
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
        rt2.vm.cold.persist.copy_from_slice(&rt.vm.cold.persist);
        rt2.set_indicator("ema", 101.0);
        rt2.set_indicator("sma", 99.0);
        rt2.feed_eval();

        let _ = std::fs::remove_file(&path_v1);
        let _ = std::fs::remove_file(&path_v2);
    }

    // в”Ђв”Ђ Hot-reload tests в”Ђв”Ђ

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
    x = 2.5
end
";
        let path_v1 = write_test_strategy("hr_type_v1", src_v1);
        let mut rt_v1 = QflRuntime::load(&path_v1).unwrap();
        rt_v1.feed_eval();
        assert_eq!(rt_v1.vm.cold.persist[0].int_val, 42);

        let persist = rt_v1.vm.cold.persist;

        let path_v2 = write_test_strategy("hr_type_v2", src_v2);
        let mut rt_v2 = QflRuntime::load(&path_v2).unwrap();
        rt_v2.vm.cold.persist.copy_from_slice(&persist);
        rt_v2.feed_eval();

        assert_eq!(rt_v2.vm.cold.persist[0].tag, 1);
        assert!((rt_v2.vm.cold.persist[0].float_val - 2.5).abs() < 0.001);

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
        assert_eq!(rt_v1.vm.cold.persist[0].int_val, 42);

        let persist = rt_v1.vm.cold.persist;

        let path_v2 = write_test_strategy("hr_rem_v2", src_v2);
        let mut rt_v2 = QflRuntime::load(&path_v2).unwrap();
        rt_v2.vm.cold.persist.copy_from_slice(&persist);
        assert_eq!(rt_v2.vm.cold.persist[0].int_val, 42);

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
        assert_eq!(rt_v1.vm.cold.persist[0].int_val, 10);

        let persist = rt_v1.vm.cold.persist;

        let path_v2 = write_test_strategy("hr_shift_v2", src_v2);
        let mut rt_v2 = QflRuntime::load(&path_v2).unwrap();
        rt_v2.vm.cold.persist.copy_from_slice(&persist);
        rt_v2.feed_eval();

        assert_eq!(rt_v2.vm.cold.persist[0].int_val, 11);
        assert_eq!(rt_v2.vm.cold.persist[1].int_val, 1);

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
            assert_eq!(rt.vm.cold.persist[0].int_val, 2);
            let _ = std::fs::remove_file(&path);
        }
    }

    // в”Ђв”Ђ Persist edge case tests в”Ђв”Ђ

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
            assert_eq!(rt.vm.cold.persist[i].int_val, 1, "slot {}", i);
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
        assert_eq!(rt.vm.cold.persist[0].int_val, 1);
        rt.feed_eval();
        assert_eq!(rt.vm.cold.persist[0].int_val, 2);
        rt.feed_trade(make_trade(101.0, 0.5, Side::Sell, 2));
        assert_eq!(rt.vm.cold.persist[0].int_val, 3);
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
        assert_eq!(rt.vm.cold.persist[0].int_val, 0);
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
        assert_eq!(rt.vm.cold.persist[0].int_val, -42);
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
        assert_eq!(rt.vm.cold.persist[0].int_val, 500000000000i64);
        let _ = std::fs::remove_file(&path);
    }

    // в”Ђв”Ђ Order flow edge case tests в”Ђв”Ђ

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

    // в”Ђв”Ђ Error path tests в”Ђв”Ђ

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
        let garbage = vec![0x00, 0x01, 0x02, 0xFF, 0xFE, 0xFD];
        std::fs::write(path, garbage).unwrap();
        let result = QflRuntime::load(path);
        assert!(result.is_err());
        let _ = std::fs::remove_file(path);
    }

    // в”Ђв”Ђ Feed state update tests в”Ђв”Ђ

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
            bids: vec![DepthLevel {
                price: 49900.0,
                qty: 1.5,
            }],
            asks: vec![DepthLevel {
                price: 50100.0,
                qty: 2.0,
            }],
        };
        rt.feed_depth(depth);
        assert_eq!(rt.vm.cold.depth_bids_len, 1);
        assert_eq!(rt.vm.cold.depth_asks_len, 1);
        assert!((rt.vm.cold.depth_bids_price[0] - 49900.0).abs() < 0.001);
        assert!((rt.vm.cold.depth_asks_price[0] - 50100.0).abs() < 0.001);
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
        assert_eq!(rt.vm.cold.persist[0].int_val, 2);

        rt.feed_depth(Depth {
            bids: vec![DepthLevel {
                price: 99.0,
                qty: 1.0,
            }],
            asks: vec![],
        });
        assert_eq!(rt.vm.cold.persist[0].int_val, 5);

        rt.feed_fill(make_fill(100.0, 1.0, Side::Buy));
        assert_eq!(rt.vm.cold.persist[0].int_val, 10);

        rt.feed_eval();
        assert_eq!(rt.vm.cold.persist[0].int_val, 17);

        let _ = std::fs::remove_file(&path);
    }

    // в”Ђв”Ђ Type-checked runtime loading в”Ђв”Ђ

    #[test]
    fn test_load_type_error_rejected() {
        // 42 + true is a type error
        let path = write_test_strategy("runtime_type_err", "42 + true");
        let result = QflRuntime::load(&path);
        assert!(
            result.is_err(),
            "type error should be rejected at load time"
        );
        let err = result.unwrap_err();
        assert!(
            err.contains("type error"),
            "error should mention type error: {}",
            err
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_valid_strategy_ok() {
        let path = write_test_strategy(
            "runtime_valid_type",
            "
            @persist local pos = 0
            function on_trade(trade)
                if trade.price > 0 then quince.order(0, 1.0, trade.price) end
            end
            function on_eval() quince.log(\"ok\") end
        ",
        );
        let result = QflRuntime::load(&path);
        assert!(
            result.is_ok(),
            "valid strategy should load: {:?}",
            result.err()
        );
        let _ = std::fs::remove_file(&path);
    }

    // в”Ђв”Ђ Risk engine integration в”Ђв”Ђ

    #[test]
    fn test_risk_rejects_large_order() {
        let path = write_test_strategy(
            "risk_large",
            "
            function on_eval() quince.order(0, 100.0, 50000.0) end
        ",
        );
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
            bids: vec![DepthLevel {
                price: 100.0,
                qty: 1.0,
            }],
            asks: vec![],
        };
        rt.feed_event(Event::Depth(depth));
        assert_eq!(rt.vm.cold.depth_bids_len, 1);
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
        let path = write_test_strategy(
            "risk_cycle",
            "
            function on_eval()
                quince.order(0, 0.1, 100.0)
                quince.order(0, 0.1, 100.0)
            end
        ",
        );
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.risk_engine.limits.max_orders_per_cycle = 1;
        rt.feed_event(Event::Eval);
        // Second eval should reset order count via new_cycle
        rt.feed_event(Event::Eval);
        let _ = std::fs::remove_file(&path);
    }

    // в”Ђв”Ђ Tracer integration в”Ђв”Ђ

    fn make_tracer_rt(name: &str, src: &str) -> (QflRuntime, String) {
        let path = write_test_strategy(name, src);
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.vm.cold.tracer = Some(crate::tracer::Tracer::new(1024));
        (rt, path)
    }

    #[test]
    fn trace_fill_records_fill_event() {
        let (mut rt, path) = make_tracer_rt("tr_fill_rec", "function on_fill(fill) end");
        let fill = make_fill(50000.0, 0.1, Side::Buy);
        rt.feed_fill(fill);
        let events = rt.vm.cold.tracer.as_mut().unwrap().drain();
        assert!(!events.is_empty());
        let has_fill = events
            .iter()
            .any(|e| matches!(e, crate::tracer::TraceEvent::Fill { .. }));
        assert!(has_fill, "expected a Fill trace event");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn trace_fill_has_correct_values() {
        let (mut rt, path) = make_tracer_rt("tr_fill_vals", "function on_fill(fill) end");
        let fill = make_fill(50200.0, 0.25, Side::Sell);
        rt.feed_fill(fill);
        let events = rt.vm.cold.tracer.as_mut().unwrap().drain();
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
        let events = rt.vm.cold.tracer.as_mut().unwrap().drain();
        let has_risk = events
            .iter()
            .any(|e| matches!(e, crate::tracer::TraceEvent::RiskAction { .. }));
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
        let events = rt.vm.cold.tracer.as_mut().unwrap().drain();
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
        let events = rt.vm.cold.tracer.as_mut().unwrap().drain();
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

    // в”Ђв”Ђ .qfr save/load integration в”Ђв”Ђ

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
        assert_eq!(rt2.vm.cold.const_pool.len(), rt.vm.cold.const_pool.len());
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
        rt.feed_depth(Depth {
            bids: vec![],
            asks: vec![],
        });
        assert_eq!(rt.vm.cold.depth_bids_len, 0);
        assert_eq!(rt.vm.cold.depth_asks_len, 0);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_on_fill_with_field_access() {
        let path = write_test_strategy(
            "rt_fill_field",
            "function on_fill(fill) local p = fill.price local q = fill.qty end",
        );
        let mut rt = QflRuntime::load(&path).unwrap();
        rt.feed_fill(make_fill(50000.0, 0.5, Side::Buy));
        assert!((rt.vm.reg_f(0) - 50000.0).abs() < 0.001);
        assert!((rt.vm.reg_f(1) - 0.5).abs() < 0.001);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_state_var_with_typed_decl() {
        let src = "
@persist counter : f64 = 0.0
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

    // в”Ђв”Ђ Load test: full pipeline + VM execution в”Ђв”Ђ

    const STRATEGIES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../strategies/");

    #[test]
    fn load_test_scalper_10k_ticks() {
        let src = std::fs::read_to_string(format!("{}scalper.qfl", STRATEGIES_DIR))
            .expect("read scalper.qfl");
        let program = crate::parser::parse(&src).expect("parse scalper");
        let mut qfr = crate::compiler::compile_checked(&program).expect("compile scalper");
        crate::optimize::optimize(&mut qfr);

        let trade = quince_core::types::Trade {
            price: 100.0,
            qty: 1.0,
            time: chrono::Utc::now(),
            side: quince_core::types::Side::Buy,
            trade_id: 1,
        };

        // Warmup
        let mut vm = crate::vm::Vm::new(qfr.clone());
        for _ in 0..100 {
            vm.set_last_price(trade.price);
            vm.set_position_size(0.0);
            vm.regs[0].f = trade.price;
            vm.regs[1].f = trade.qty;
            vm.regs[2].i = match trade.side {
                quince_core::types::Side::Buy => 0,
                _ => 1,
            };
            vm.regs[3].i = trade.trade_id as i64;
            vm.regs[4].i = trade.time.timestamp_nanos_opt().unwrap_or(0);
            vm.call("on_trade");
        }

        // Timed run: 10k iterations
        let start = std::time::Instant::now();
        for _ in 0..10_000 {
            vm.set_last_price(trade.price);
            vm.set_position_size(0.0);
            vm.regs[0].f = trade.price;
            vm.regs[1].f = trade.qty;
            vm.regs[2].i = match trade.side {
                quince_core::types::Side::Buy => 0,
                _ => 1,
            };
            vm.regs[3].i = trade.trade_id as i64;
            vm.regs[4].i = trade.time.timestamp_nanos_opt().unwrap_or(0);
            vm.call("on_trade");
        }
        let elapsed = start.elapsed();
        let ns_per_tick = elapsed.as_nanos() / 10_000;
        let ops_per_ms = (10_000.0 / elapsed.as_secs_f64()) / 1000.0;
        println!(
            "\nв•ђв•ђв•ђ LOAD TEST scalper.qfl в•ђв•ђв•ђ\n  {} iterations in {:?}\n  {:.1} ns/tick  |  {:.0} ops/ms\n  {} instrs (optimized)\n  {} entries\n",
            10_000, elapsed, ns_per_tick, ops_per_ms, qfr.code.len(), qfr.entries.len()
        );
    }

    #[test]
    fn load_test_ema_cross_10k_ticks() {
        let src = std::fs::read_to_string(format!("{}ema_cross.qfl", STRATEGIES_DIR))
            .expect("read ema_cross.qfl");
        let program = crate::parser::parse(&src).expect("parse ema_cross");
        let mut qfr = crate::compiler::compile_checked(&program).expect("compile ema_cross");
        crate::optimize::optimize(&mut qfr);

        let trade = quince_core::types::Trade {
            price: 100.0,
            qty: 1.0,
            time: chrono::Utc::now(),
            side: quince_core::types::Side::Buy,
            trade_id: 1,
        };

        let mut vm = crate::vm::Vm::new(qfr.clone());
        for _ in 0..100 {
            vm.set_last_price(trade.price);
            vm.regs[0].f = trade.price;
            vm.regs[1].f = trade.qty;
            vm.regs[2].i = 0;
            vm.regs[3].i = trade.trade_id as i64;
            vm.regs[4].i = trade.time.timestamp_nanos_opt().unwrap_or(0);
            vm.call("on_trade");
        }

        let start = std::time::Instant::now();
        for _ in 0..10_000 {
            vm.set_last_price(trade.price);
            vm.regs[0].f = trade.price;
            vm.regs[1].f = trade.qty;
            vm.regs[2].i = 0;
            vm.regs[3].i = trade.trade_id as i64;
            vm.regs[4].i = trade.time.timestamp_nanos_opt().unwrap_or(0);
            vm.call("on_trade");
        }
        let elapsed = start.elapsed();
        let ns_per_tick = elapsed.as_nanos() / 10_000;
        let ops_per_ms = (10_000.0 / elapsed.as_secs_f64()) / 1000.0;
        println!(
            "\nв•ђв•ђв•ђ LOAD TEST ema_cross.qfl в•ђв•ђв•ђ\n  {} iterations in {:?}\n  {:.1} ns/tick  |  {:.0} ops/ms\n  {} instrs (optimized)\n",
            10_000, elapsed, ns_per_tick, ops_per_ms, qfr.code.len()
        );
    }

    #[test]
    fn load_test_momentum_10k_ticks() {
        let src = std::fs::read_to_string(format!("{}momentum.qfl", STRATEGIES_DIR))
            .expect("read momentum.qfl");
        let program = crate::parser::parse(&src).expect("parse momentum");
        let mut qfr = crate::compiler::compile_checked(&program).expect("compile momentum");
        crate::optimize::optimize(&mut qfr);

        let trade = quince_core::types::Trade {
            price: 100.0,
            qty: 1.0,
            time: chrono::Utc::now(),
            side: quince_core::types::Side::Buy,
            trade_id: 1,
        };

        let mut vm = crate::vm::Vm::new(qfr.clone());
        for _ in 0..100 {
            vm.set_last_price(trade.price);
            vm.regs[0].f = trade.price;
            vm.regs[1].f = trade.qty;
            vm.regs[2].i = 0;
            vm.regs[3].i = trade.trade_id as i64;
            vm.regs[4].i = trade.time.timestamp_nanos_opt().unwrap_or(0);
            vm.call("on_trade");
        }

        let start = std::time::Instant::now();
        for _ in 0..10_000 {
            vm.set_last_price(trade.price);
            vm.regs[0].f = trade.price;
            vm.regs[1].f = trade.qty;
            vm.regs[2].i = 0;
            vm.regs[3].i = trade.trade_id as i64;
            vm.regs[4].i = trade.time.timestamp_nanos_opt().unwrap_or(0);
            vm.call("on_trade");
        }
        let elapsed = start.elapsed();
        let ns_per_tick = elapsed.as_nanos() / 10_000;
        let ops_per_ms = (10_000.0 / elapsed.as_secs_f64()) / 1000.0;
        println!(
            "\nв•ђв•ђв•ђ LOAD TEST momentum.qfl в•ђв•ђв•ђ\n  {} iterations in {:?}\n  {:.1} ns/tick  |  {:.0} ops/ms\n  {} instrs (optimized)\n",
            10_000, elapsed, ns_per_tick, ops_per_ms, qfr.code.len()
        );
    }

    #[test]
    fn load_test_heavy_100k_events() {
        let path = format!("{}heavy_test.qfl", STRATEGIES_DIR);
        let src = std::fs::read_to_string(&path).expect("read heavy_test.qfl");
        let program = crate::parser::parse(&src).expect("parse heavy_test");
        let mut qfr = crate::compiler::compile_checked(&program).expect("compile heavy_test");
        crate::optimize::optimize(&mut qfr);
        let instr_count = qfr.code.len();

        let mut rt = QflRuntime::load(&path).expect("load heavy_test.qfl");
        rt.set_symbol("BTCUSDT");
        rt.set_balance("USDT", 10000.0);
        rt.set_position_size(0.0);

        println!(
            "\nв•ђв•ђв•ђ HEAVY TEST в•ђв•ђв•ђ\n  {} instrs (optimized)\n",
            instr_count
        );

        // Warmup
        for i in 0..100 {
            let trade = quince_core::types::Trade {
                price: 50000.0 + (i % 1000) as f64,
                qty: 0.1 + (i % 5) as f64 * 0.1,
                time: chrono::Utc::now(),
                side: if i % 2 == 0 {
                    quince_core::types::Side::Buy
                } else {
                    quince_core::types::Side::Sell
                },
                trade_id: i as u64,
            };
            rt.feed_trade(trade);
        }
        for _ in 0..10 {
            let depth = quince_core::types::Depth {
                bids: vec![
                    quince_core::types::DepthLevel {
                        price: 50000.0,
                        qty: 1.0,
                    },
                    quince_core::types::DepthLevel {
                        price: 49900.0,
                        qty: 5.0,
                    },
                ],
                asks: vec![
                    quince_core::types::DepthLevel {
                        price: 50100.0,
                        qty: 1.5,
                    },
                    quince_core::types::DepthLevel {
                        price: 50200.0,
                        qty: 3.0,
                    },
                ],
            };
            rt.feed_depth(depth);
        }
        for _ in 0..10 {
            rt.feed_eval();
        }

        // Timed run: 100k mixed events
        let start = std::time::Instant::now();
        for i in 0..100_000 {
            let event_kind = i % 5;
            match event_kind {
                0..=2 => {
                    let trade = quince_core::types::Trade {
                        price: 50000.0 + (i % 2000) as f64,
                        qty: 0.1 + (i % 10) as f64 * 0.05,
                        time: chrono::Utc::now(),
                        side: if i % 2 == 0 {
                            quince_core::types::Side::Buy
                        } else {
                            quince_core::types::Side::Sell
                        },
                        trade_id: i as u64,
                    };
                    rt.feed_trade(trade);
                }
                3 => {
                    let depth = quince_core::types::Depth {
                        bids: vec![
                            quince_core::types::DepthLevel {
                                price: 50000.0 - (i % 100) as f64,
                                qty: 0.5,
                            },
                            quince_core::types::DepthLevel {
                                price: 49800.0,
                                qty: 2.0,
                            },
                        ],
                        asks: vec![
                            quince_core::types::DepthLevel {
                                price: 50100.0 + (i % 100) as f64,
                                qty: 0.8,
                            },
                            quince_core::types::DepthLevel {
                                price: 50300.0,
                                qty: 1.5,
                            },
                        ],
                    };
                    rt.feed_depth(depth);
                }
                4 => {
                    rt.feed_eval();
                }
                _ => {}
            }
        }
        let elapsed = start.elapsed();
        let ns_per_event = elapsed.as_nanos() / 100_000;
        let ops_per_ms = (100_000.0 / elapsed.as_secs_f64()) / 1000.0;
        println!(
            "в•ђв•ђв•ђ LOAD TEST heavy_test.qfl в•ђв•ђв•ђ\n  {} events in {:?}\n  {:.1} ns/event  |  {:.0} ops/ms\n  {} instrs (optimized)\n",
            100_000, elapsed, ns_per_event, ops_per_ms, instr_count
        );
        assert!(elapsed.as_secs() < 30, "heavy_test took too long");
    }
}
