use crate::indicators::{parse_using, IndicatorBank};
use crate::orders::{OrderManager, PendingStatus};
use quince_core::types::*;
use quince_exchange::r#trait::{Exchange, ExchangeError, StreamMsg};
use quince_logger::TradeLog;
use quince_qfl::runtime::QflRuntime;
use quince_risk::RiskControls;
use std::sync::Arc;
use std::time::{Duration, Instant};

const IDLE_SLEEP_MS: u64 = 1;
const MAX_STREAM_MSGS_PER_ITER: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("Exchange error: {0}")]
    Exchange(#[from] ExchangeError),
    #[error("Strategy error: {0}")]
    Strategy(String),
    #[error("Risk rejected: {0}")]
    RiskRejected(String),
    #[error("Order timeout: {0}")]
    OrderTimeout(String),
}

const ORDER_TIMEOUT: Duration = Duration::from_secs(30);
const EVAL_INTERVAL: Duration = Duration::from_secs(1);
const ACCOUNT_SYNC_INTERVAL: Duration = Duration::from_secs(10);

pub struct Engine<E: Exchange> {
    exchange: E,
    symbols: Vec<String>,

    orders_rx: crossbeam_channel::Receiver<Order>,

    qfl: QflRuntime,

    risk: RiskControls,
    logger: TradeLog,

    order_manager: OrderManager,
    indicators: IndicatorBank,

    last_price: f64,
    daily_pnl: f64,
    peak_equity: f64,
    // Account state for equity check (Vec + linear search, N ≤ 5)
    balance_names: Vec<String>,
    balance_values: Vec<f64>,
    position: Option<Position>,

    next_eval: Instant,
    next_account: Instant,

    // Cached indicator slots (avoid HashMap lookup on every eval)
    entry_price_slot: u16,
    unrealized_pnl_slot: u16,

    #[cfg(feature = "profiling")]
    profiling_frame: u64,
}

impl<E: Exchange> Engine<E> {
    pub fn new(
        exchange: E,
        symbols: &[String],
        strategy_path: &str,
        risk: RiskControls,
        log_path: &str,
    ) -> Result<Self, EngineError> {
        let (orders_tx, orders_rx) = crossbeam_channel::unbounded();

        // Load QFL strategy (.qfl = compile+optimize, .qfr = pre-compiled)
        let is_qfr = strategy_path.ends_with(".qfr");
        let mut qfl = if is_qfr {
            QflRuntime::load_qfr(strategy_path).map_err(|e| EngineError::Strategy(e))?
        } else {
            let qfl = QflRuntime::load(strategy_path).map_err(|e| EngineError::Strategy(e))?;
            let qfr_path = strategy_path.replace(".qfl", ".qfr");
            qfl.save_qfr(&qfr_path)
                .map_err(|e| EngineError::Strategy(format!("save .qfr: {}", e)))?;
            tracing::info!("optimized bytecode saved to {qfr_path}");
            qfl
        };

        tracing::info!("QFL VM loaded: {strategy_path}");

        // Read source for --USING directives (from .qfl companion for .qfr)
        let src_path = if is_qfr {
            let qfl_path = strategy_path.replace(".qfr", ".qfl");
            if std::path::Path::new(&qfl_path).exists() {
                qfl_path
            } else {
                String::new()
            }
        } else {
            strategy_path.to_string()
        };
        let src = if src_path.is_empty() {
            String::new()
        } else {
            std::fs::read_to_string(&src_path)
                .map_err(|e| EngineError::Strategy(format!("read {}: {}", src_path, e)))?
        };
        tracing::info!(
            "strategy loaded: {strategy_path} ({} lines)",
            src.lines().count()
        );

        let ind_cfg = parse_using(&src);
        for entry in &ind_cfg {
            tracing::info!(
                "  indicator: {} params={:?} buffer={}",
                entry.name,
                entry.params,
                entry.buffer
            );
        }
        tracing::info!("parsed {} indicator directives", ind_cfg.len());

        let mut indicators = IndicatorBank::new(&ind_cfg);

        // Phase 4g: pre-assign indicator slots — zero HashMap lookups in hot path
        let synthetic_names = [
            "price",
            "volume_delta",
            "avg_trade_size",
            "trade_count",
            "bid_depth",
            "ask_depth",
            "depth_imbalance",
            "entry_price",
            "unrealized_pnl",
        ];
        for entry in &ind_cfg {
            let slot = qfl.ensure_indicator_slot(&entry.name);
            indicators.set_name_to_slot(&entry.name, slot);
            match entry.name.as_str() {
                "macd" => {
                    indicators
                        .set_name_to_slot("macd.signal", qfl.ensure_indicator_slot("macd.signal"));
                    indicators.set_name_to_slot(
                        "macd.histogram",
                        qfl.ensure_indicator_slot("macd.histogram"),
                    );
                }
                "bb" => {
                    indicators
                        .set_name_to_slot("bb.middle", qfl.ensure_indicator_slot("bb.middle"));
                    indicators.set_name_to_slot("bb.upper", qfl.ensure_indicator_slot("bb.upper"));
                    indicators.set_name_to_slot("bb.lower", qfl.ensure_indicator_slot("bb.lower"));
                    indicators.set_name_to_slot(
                        "bb.bandwidth",
                        qfl.ensure_indicator_slot("bb.bandwidth"),
                    );
                }
                "kc" => {
                    indicators
                        .set_name_to_slot("kc.middle", qfl.ensure_indicator_slot("kc.middle"));
                    indicators.set_name_to_slot("kc.upper", qfl.ensure_indicator_slot("kc.upper"));
                    indicators.set_name_to_slot("kc.lower", qfl.ensure_indicator_slot("kc.lower"));
                }
                _ => {}
            }
        }
        for name in &synthetic_names {
            let slot = qfl.ensure_indicator_slot(name);
            indicators.set_name_to_slot(name, slot);
        }

        // Cache frequently-used indicator slots (no HashMap lookup in hot path)
        let entry_price_slot = qfl.ensure_indicator_slot("entry_price");
        let unrealized_pnl_slot = qfl.ensure_indicator_slot("unrealized_pnl");

        // Finalize VM const→slot lookups (replaces HashMap+String in vm_getind/vm_getbal)
        qfl.finalize_vm_init();

        tracing::info!("indicator bank ready: {} indicators", ind_cfg.len());
        drop(src);

        // Wire QFL runtime to send orders through the engine channel
        qfl.set_order_sender(orders_tx);
        qfl.set_symbol(symbols.first().map(|s| s.as_str()).unwrap_or(""));

        tracing::info!("symbols: {:?}, log: {log_path}", symbols);
        tracing::info!(
            "risk: max_pos={} max_dd={}% max_order_freq={}/s max_daily_loss={}",
            risk.max_position_size,
            risk.max_drawdown * 100.0,
            risk.max_order_freq,
            risk.max_daily_loss,
        );

        let logger = TradeLog::new(log_path);

        Ok(Self {
            exchange,
            symbols: symbols.to_vec(),
            orders_rx,
            qfl,
            risk,
            logger,
            order_manager: OrderManager::new(),
            indicators,
            last_price: 0.0,
            daily_pnl: 0.0,
            peak_equity: 0.0,
            balance_names: Vec::new(),
            balance_values: Vec::new(),
            position: None,
            next_eval: Instant::now() + EVAL_INTERVAL,
            next_account: Instant::now() + ACCOUNT_SYNC_INTERVAL,
            entry_price_slot,
            unrealized_pnl_slot,
            #[cfg(feature = "profiling")]
            profiling_frame: 0,
        })
    }

    pub async fn run(&mut self) -> Result<(), EngineError> {
        let stream = self.exchange.subscribe(&self.symbols).await?;
        let rx = stream.rx;

        tracing::info!(
            "engine loop starting — {} symbol(s) subscribed, {} stream(s) active",
            self.symbols.len(),
            self.symbols.len() * 2,
        );

        loop {
            #[cfg(feature = "profiling")]
            {
                puffin::GlobalProfiler::lock().new_frame();
                self.profiling_frame += 1;
            }

            let mut did_work = false;
            let now = Instant::now();

            // Priority 2: Periodic eval — check FIRST to prevent starvation
            if now >= self.next_eval {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("Eval");
                did_work = true;
                self.on_eval().await;
                self.next_eval = now + EVAL_INTERVAL;
            }

            // Priority 0: Stream messages (market data — most latency-sensitive)
            // Limit to MAX_STREAM_MSGS_PER_ITER to prevent starvation of lower priorities
            let mut stream_count = 0;
            while let Ok(msg) = rx.try_recv() {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("StreamMsg");
                did_work = true;
                self.on_stream_msg(msg).await;
                stream_count += 1;
                if stream_count >= MAX_STREAM_MSGS_PER_ITER {
                    break;
                }
            }

            // Priority 1: Strategy orders (from QFL VM / flush_pending_order)
            while let Ok(order) = self.orders_rx.try_recv() {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("StrategyOrder");
                did_work = true;
                self.on_strategy_order(order).await;
            }

            // Priority 3: Periodic account sync
            if now >= self.next_account {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("AccountSync");
                did_work = true;
                self.sync_account().await;
                self.next_account = now + ACCOUNT_SYNC_INTERVAL;
            }

            // Always: check timeouts and SL/TP
            {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("CheckTimeouts");
                self.check_timeouts().await;
            }
            {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("CheckSlTp");
                self.check_sl_tp().await;
            }

            // Backoff: sleep when idle to avoid busy spin
            if !did_work {
                tokio::time::sleep(Duration::from_millis(IDLE_SLEEP_MS)).await;
            }
        }
    }

    async fn on_stream_msg(&mut self, msg: StreamMsg) {
        match msg {
            StreamMsg::Trade(trade) => {
                self.last_price = trade.price;
                for &(slot, v) in self.indicators.on_trade(&trade) {
                    self.qfl.set_indicator_by_slot(slot, v);
                }
                self.qfl.feed_trade(trade);
            }
            StreamMsg::Depth(depth) => {
                for &(slot, v) in self.indicators.on_depth(&depth) {
                    self.qfl.set_indicator_by_slot(slot, v);
                }
                self.qfl.feed_depth(depth);
            }
            StreamMsg::MarkPrice { price, .. } => {
                self.last_price = price;
            }
            StreamMsg::OrderUpdate(fill) => {
                let cid = self
                    .order_manager
                    .find_client_by_exchange_id(&fill.order_id);
                if let Some(cid) = cid {
                    let cid = cid.to_string();
                    self.order_manager.update_fill(&cid, fill.qty, fill.price);
                    self.logger.log_fill(&fill);
                    self.daily_pnl -= fill.fee;
                    self.qfl.feed_fill(fill);
                }
            }
            StreamMsg::AccountUpdate(info) => {
                for b in &info.balances {
                    self.set_balance(&b.asset, b.wallet);
                    self.qfl.set_balance(&b.asset, b.wallet);
                }
                if let Some(pos) = info
                    .positions
                    .into_iter()
                    .find(|p| p.symbol == self.symbols.first().cloned().unwrap_or_default())
                {
                    self.position = Some(pos.clone());
                    self.qfl.set_position_size(pos.size);
                }
            }
            StreamMsg::OpenInterest { .. } | StreamMsg::ForceOrder(_) => {}
        }
    }

    /// Vec-based balance store — linear search over N ≤ 5.
    fn set_balance(&mut self, name: &str, val: f64) {
        if let Some(i) = self.balance_names.iter().position(|n| n == name) {
            self.balance_values[i] = val;
        } else {
            self.balance_names.push(name.to_string());
            self.balance_values.push(val);
        }
    }

    async fn on_strategy_order(&mut self, order: Order) {
        if let Err(reason) = self
            .risk
            .check_order(&order, self.daily_pnl, self.peak_equity)
        {
            tracing::warn!("risk rejected order: {}", reason);
            return;
        }

        let client_id = self.order_manager.register(order);
        if let Some(po) = self.order_manager.get(&client_id) {
            match self.exchange.place_order(po.order.clone()).await {
                Ok(order_id) => {
                    self.order_manager.mark_placed(&client_id, order_id);
                    self.risk.record_trade();
                }
                Err(e) => {
                    self.order_manager.mark_failed(&client_id, e.to_string());
                }
            }
        }
    }

    async fn on_eval(&mut self) {
        // Push latest balances/position to QFL VM before eval
        for (name, bal) in self.balance_names.iter().zip(&self.balance_values) {
            self.qfl.set_balance(name, *bal);
        }
        if let Some(pos) = &self.position {
            self.qfl.set_position_size(pos.size);
            self.qfl
                .set_indicator_by_slot(self.entry_price_slot, pos.entry_price);
            self.qfl
                .set_indicator_by_slot(self.unrealized_pnl_slot, pos.unrealized_pnl);
        }
        self.qfl.feed_eval();
        self.equity_check();
    }

    async fn sync_account(&mut self) {
        match self.exchange.account_info().await {
            Ok(info) => {
                for b in &info.balances {
                    self.set_balance(&b.asset, b.wallet);
                    self.qfl.set_balance(&b.asset, b.wallet);
                }
                if let Some(pos) = info
                    .positions
                    .into_iter()
                    .find(|p| p.symbol == self.symbols.first().cloned().unwrap_or_default())
                {
                    self.position = Some(pos.clone());
                    self.qfl.set_position_size(pos.size);
                }
            }
            Err(e) => tracing::warn!("account sync failed: {}", e),
        }
    }

    async fn check_timeouts(&mut self) {
        if !self.order_manager.has_pending() {
            return;
        }
        let now = Instant::now();
        let timed_out: Vec<String> = self
            .order_manager
            .pending_order_ids()
            .into_iter()
            .filter(|cid| {
                if let Some(po) = self.order_manager.get(cid) {
                    now.duration_since(po.placed_at) > ORDER_TIMEOUT
                } else {
                    false
                }
            })
            .collect();

        for cid in timed_out {
            if let Some(po) = self.order_manager.get(&cid) {
                let symbol = po.order.symbol.as_ref();
                if let PendingStatus::Placed { order_id } = &po.status {
                    let _ = self.exchange.cancel_order(symbol, order_id).await;
                }
                self.order_manager.cancel(&cid);
                tracing::warn!("order timed out: {}", cid);
            }
        }

        self.order_manager.cleanup_filled();
    }

    async fn check_sl_tp(&mut self) {
        let price = self.last_price;
        if price <= 0.0 {
            return;
        }

        let triggered: Vec<(String, Side, f64)> = self
            .order_manager
            .active_sl_tp()
            .into_iter()
            .filter_map(|stop| {
                if stop.side == Side::Sell {
                    if let Some(sl) = stop.stop_loss {
                        if price <= sl {
                            return Some((stop.client_id, stop.side, stop.qty));
                        }
                    }
                    if let Some(tp) = stop.take_profit {
                        if price >= tp {
                            return Some((stop.client_id, stop.side, stop.qty));
                        }
                    }
                } else {
                    if let Some(sl) = stop.stop_loss {
                        if price >= sl {
                            return Some((stop.client_id, stop.side, stop.qty));
                        }
                    }
                    if let Some(tp) = stop.take_profit {
                        if price <= tp {
                            return Some((stop.client_id, stop.side, stop.qty));
                        }
                    }
                }
                None
            })
            .collect();

        for (cid, side, qty) in triggered {
            let close = Order {
                symbol: self.symbols.first().map(|s| Arc::<str>::from(s.as_str())).unwrap_or_else(|| Arc::from("")),
                side,
                qty,
                price: None,
                order_type: OrderType::Market,
                reduce_only: true,
                stop_loss: None,
                take_profit: None,
            };
            match self.exchange.place_order(close).await {
                Ok(id) => {
                    self.order_manager.deactivate_sl_tp(&cid);
                    tracing::info!("SL/TP triggered for {cid}: close {side:?} {qty} order={id}");
                }
                Err(e) => {
                    tracing::warn!("SL/TP close order failed for {cid}: {e}");
                }
            }
        }
    }

    fn equity_check(&mut self) {
        let usdt = self
            .balance_names
            .iter()
            .position(|n| n == "USDT")
            .and_then(|i| self.balance_values.get(i).copied())
            .unwrap_or(0.0);
        let equity = usdt
            + self
                .position
                .as_ref()
                .map(|p| p.unrealized_pnl)
                .unwrap_or(0.0);

        if equity > self.peak_equity {
            self.peak_equity = equity;
        }

        if self.peak_equity > 0.0 {
            let drawdown = (self.peak_equity - equity) / self.peak_equity;
            if drawdown > self.risk.max_drawdown {
                tracing::warn!(
                    "drawdown {:.2}% exceeds limit {:.2}%",
                    drawdown * 100.0,
                    self.risk.max_drawdown * 100.0
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_error_exchange_display() {
        let msg = EngineError::Exchange(ExchangeError::Ws("timeout".into())).to_string();
        assert!(msg.contains("Exchange error"));
    }

    #[test]
    fn engine_error_strategy_display() {
        let msg = EngineError::Strategy("compilation failed".into()).to_string();
        assert!(msg.contains("compilation failed"));
    }

    #[test]
    fn engine_error_risk_display() {
        let msg = EngineError::RiskRejected("max drawdown exceeded".into()).to_string();
        assert!(msg.contains("Risk rejected"));
    }

    #[test]
    fn engine_error_order_timeout_display() {
        let msg = EngineError::OrderTimeout("order 123".into()).to_string();
        assert!(msg.contains("Order timeout"));
    }

    #[test]
    fn constants_defined() {
        assert_eq!(ORDER_TIMEOUT, Duration::from_secs(30));
        assert_eq!(EVAL_INTERVAL, Duration::from_secs(1));
        assert_eq!(ACCOUNT_SYNC_INTERVAL, Duration::from_secs(10));
        assert_eq!(IDLE_SLEEP_MS, 1);
    }

    #[test]
    fn engine_error_from_exchange_error() {
        let e: EngineError = ExchangeError::Ws("fail".into()).into();
        assert!(matches!(e, EngineError::Exchange(_)));
    }
}
