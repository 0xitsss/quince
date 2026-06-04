use crate::indicators::{parse_using, IndicatorBank};
use crate::orders::{OrderManager, PendingStatus};
use quince_core::types::*;
use quince_exchange::r#trait::{Exchange, ExchangeError, StreamMsg};
use quince_logger::TradeLog;
use quince_qfl::runtime::QflRuntime;
use quince_risk::RiskControls;
use std::collections::HashMap;
use std::time::{Duration, Instant};

const IDLE_SLEEP_MS: u64 = 1;

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
    order_timestamps: Vec<Instant>,

    // Account state for equity check
    balances: HashMap<String, f64>,
    position: Option<Position>,

    next_eval: Instant,
    next_account: Instant,

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

        // Load QFL strategy
        let mut qfl = QflRuntime::load(strategy_path)
            .map_err(|e| EngineError::Strategy(e))?;

        tracing::info!("QFL VM loaded: {strategy_path}");

        // Read source for --USING directives
        let src = std::fs::read_to_string(strategy_path)
            .map_err(|e| EngineError::Strategy(format!("read {}: {}", strategy_path, e)))?;
        tracing::info!("strategy loaded: {strategy_path} ({} lines)", src.lines().count());

        let ind_cfg = parse_using(&src);
        for entry in &ind_cfg {
            tracing::info!("  indicator: {} params={:?} buffer={}", entry.name, entry.params, entry.buffer);
        }
        tracing::info!("parsed {} indicator directives", ind_cfg.len());

        let indicators = IndicatorBank::new(&ind_cfg);
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
            order_timestamps: Vec::new(),
            balances: HashMap::new(),
            position: None,
            next_eval: Instant::now() + EVAL_INTERVAL,
            next_account: Instant::now() + ACCOUNT_SYNC_INTERVAL,
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

            // Priority 0: Stream messages (market data — most latency-sensitive)
            while let Ok(msg) = rx.try_recv() {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("StreamMsg");
                did_work = true;
                self.on_stream_msg(msg).await;
            }

            // Priority 1: Strategy orders (from QFL VM / flush_pending_order)
            while let Ok(order) = self.orders_rx.try_recv() {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("StrategyOrder");
                did_work = true;
                self.on_strategy_order(order).await;
            }

            // Priority 2: Periodic eval
            let now = Instant::now();
            if now >= self.next_eval {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("Eval");
                did_work = true;
                self.on_eval().await;
                self.next_eval = now + EVAL_INTERVAL;
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
                let ind_vals = self.indicators.on_trade(&trade).to_vec();
                for &(k, v) in &ind_vals {
                    self.qfl.set_indicator(k, v);
                }
                // Push trade to QFL VM
                self.qfl.feed_trade(trade);
            }
            StreamMsg::Depth(depth) => {
                let ind_vals = self.indicators.on_depth(&depth).to_vec();
                for &(k, v) in &ind_vals {
                    self.qfl.set_indicator(k, v);
                }
                self.qfl.feed_depth(depth);
            }
            StreamMsg::MarkPrice { price, .. } => {
                self.last_price = price;
            }
            StreamMsg::OrderUpdate(fill) => {
                let cid = self.order_manager.find_client_by_exchange_id(&fill.order_id);
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
                    self.balances.insert(b.asset.clone(), b.wallet);
                    self.qfl.set_balance(&b.asset, b.wallet);
                }
                if let Some(pos) = info.positions.into_iter().find(|p| p.symbol == self.symbols.first().cloned().unwrap_or_default()) {
                    self.position = Some(pos.clone());
                    self.qfl.set_position_size(pos.size);
                }
            }
            StreamMsg::OpenInterest { .. } | StreamMsg::ForceOrder(_) => {}
        }
    }

    async fn on_strategy_order(&mut self, order: Order) {
        if let Err(reason) = self.risk.check_order(
            &order,
            self.daily_pnl,
            self.peak_equity,
        ) {
            tracing::warn!("risk rejected order: {}", reason);
            return;
        }

        let client_id = self.order_manager.register(order);
        if let Some(po) = self.order_manager.get(&client_id) {
            match self.exchange.place_order(po.order.clone()).await {
                Ok(order_id) => {
                    self.order_manager.mark_placed(&client_id, order_id);
                    self.order_timestamps.push(Instant::now());
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
        for (asset, bal) in &self.balances {
            self.qfl.set_balance(asset, *bal);
        }
        if let Some(pos) = &self.position {
            self.qfl.set_position_size(pos.size);
            self.qfl.set_indicator("entry_price", pos.entry_price);
            self.qfl.set_indicator("unrealized_pnl", pos.unrealized_pnl);
        }
        self.qfl.feed_eval();
        self.equity_check();
    }

    async fn sync_account(&mut self) {
        match self.exchange.account_info().await {
            Ok(info) => {
                for b in &info.balances {
                    self.balances.insert(b.asset.clone(), b.wallet);
                    self.qfl.set_balance(&b.asset, b.wallet);
                }
                if let Some(pos) = info.positions.into_iter().find(|p| p.symbol == self.symbols.first().cloned().unwrap_or_default()) {
                    self.position = Some(pos.clone());
                    self.qfl.set_position_size(pos.size);
                }
            }
            Err(e) => tracing::warn!("account sync failed: {}", e),
        }
    }

    async fn check_timeouts(&mut self) {
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
                let symbol = &po.order.symbol;
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
        if price <= 0.0 { return; }

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
                symbol: self.symbols.first().cloned().unwrap_or_default(),
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
        let equity = self.balances.get("USDT").copied().unwrap_or(0.0)
            + self.position.as_ref().map(|p| p.unrealized_pnl).unwrap_or(0.0);

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
