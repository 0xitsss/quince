use crate::indicators::{parse_using, IndicatorBank};
use crate::orders::{OrderManager, PendingStatus};
use quince_core::types::*;
use quince_exchange::r#trait::{Exchange, ExchangeError, StreamMsg};
use quince_logger::TradeLog;
use quince_risk::RiskControls;
use quince_strategy::runtime::{spawn_strategy, LuaEvent, StrategyCtx};
use std::collections::HashMap;
use crossbeam_channel;
use std::sync::{Arc, Mutex};
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

const MAX_TRADES: usize = 10_000;
const ORDER_TIMEOUT: Duration = Duration::from_secs(30);
const EVAL_INTERVAL: Duration = Duration::from_secs(1);
const ACCOUNT_SYNC_INTERVAL: Duration = Duration::from_secs(10);

pub struct Engine<E: Exchange> {
    exchange: E,
    symbols: Vec<String>,

    event_tx: crossbeam_channel::Sender<LuaEvent>,
    orders_rx: crossbeam_channel::Receiver<Order>,
    ctx: Arc<Mutex<StrategyCtx>>,

    risk: RiskControls,
    logger: TradeLog,

    order_manager: OrderManager,
    indicators: IndicatorBank,

    last_price: f64,
    daily_pnl: f64,
    peak_equity: f64,
    order_timestamps: Vec<Instant>,

    next_eval: Instant,
    next_account: Instant,

    // Profiling / latency stats (reserved)
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
        let (event_tx, event_rx) = crossbeam_channel::bounded(1024);
        let (orders_tx, orders_rx) = crossbeam_channel::unbounded();

        let ctx = StrategyCtx {
            trades: Vec::with_capacity(MAX_TRADES),
            depth: None,
            indicators: HashMap::new(),
            balance: HashMap::new(),
            position: None,
            orders_tx,
        };
        let ctx = Arc::new(Mutex::new(ctx));

        spawn_strategy(strategy_path, event_rx, Arc::clone(&ctx))
            .map_err(|e| EngineError::Strategy(e))?;
        tracing::info!("Lua VM spawned: {strategy_path}");

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

        // Pre-populate ctx.indicators keys so Lua can read them immediately
        {
            let mut ctx = ctx.lock().unwrap();
            for entry in &ind_cfg {
                match entry.name.as_str() {
                    "macd" => { ctx.indicators.insert("macd", 0.0); ctx.indicators.insert("macd.signal", 0.0); ctx.indicators.insert("macd.histogram", 0.0); }
                    "bb" => { ctx.indicators.insert("bb.middle", 0.0); ctx.indicators.insert("bb.upper", 0.0); ctx.indicators.insert("bb.lower", 0.0); ctx.indicators.insert("bb.bandwidth", 0.0); }
                    "kc" => { ctx.indicators.insert("kc.middle", 0.0); ctx.indicators.insert("kc.upper", 0.0); ctx.indicators.insert("kc.lower", 0.0); }
                    _ => {}
                }
            }
            ctx.indicators.insert("price", 0.0);
            ctx.indicators.insert("volume_delta", 0.0);
            ctx.indicators.insert("avg_trade_size", 0.0);
            ctx.indicators.insert("trade_count", 0.0);
            ctx.indicators.insert("bid_depth", 0.0);
            ctx.indicators.insert("ask_depth", 0.0);
            ctx.indicators.insert("depth_imbalance", 0.0);
        }

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
            event_tx,
            orders_rx,
            ctx,
            risk,
            logger,
            order_manager: OrderManager::new(),
            indicators,
            last_price: 0.0,
            daily_pnl: 0.0,
            peak_equity: 0.0,
            order_timestamps: Vec::new(),
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

            // Priority 1: Strategy orders
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
                let ind_vals = self.indicators.on_trade(&trade);
                {
                    let mut ctx = self.ctx.lock().unwrap();
                    ctx.trades.push(trade);
                    for &(k, v) in ind_vals {
                        ctx.indicators.insert(k, v);
                    }
                    let overflow = ctx.trades.len().saturating_sub(MAX_TRADES);
                    if overflow > 0 {
                        ctx.trades.drain(0..overflow);
                    }
                }
                let _ = self.event_tx.send(LuaEvent::Trade(trade));
            }
            StreamMsg::Depth(depth) => {
                let ind_vals = self.indicators.on_depth(&depth);
                {
                    let mut ctx = self.ctx.lock().unwrap();
                    for &(k, v) in ind_vals {
                        ctx.indicators.insert(k, v);
                    }
                    ctx.depth = Some(depth.clone());
                }
                let _ = self.event_tx.send(LuaEvent::Depth(depth));
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
                    let _ = self.event_tx.send(LuaEvent::Fill(fill));
                }
            }
            StreamMsg::AccountUpdate(info) => {
                let mut ctx = self.ctx.lock().unwrap();
                for b in &info.balances {
                    ctx.balance.insert(b.asset.clone(), b.wallet);
                }
                ctx.position = info.positions.into_iter().find(|p| p.symbol == self.symbols.first().cloned().unwrap_or_default());
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
        let _ = self.event_tx.send(LuaEvent::Eval);
        self.equity_check();
    }

    async fn sync_account(&mut self) {
        match self.exchange.account_info().await {
            Ok(info) => {
                let mut ctx = self.ctx.lock().unwrap();
                for b in &info.balances {
                    ctx.balance.insert(b.asset.clone(), b.wallet);
                }
                ctx.position = info.positions.into_iter().find(|p| p.symbol == self.symbols.first().cloned().unwrap_or_default());
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
                // Long (close_side=Sell): SL below entry, TP above
                // Short (close_side=Buy): SL above entry, TP below
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
        let ctx = self.ctx.lock().unwrap();
        let equity = ctx.balance.get("USDT").copied().unwrap_or(0.0)
            + ctx
                .position
                .as_ref()
                .map(|p| p.unrealized_pnl)
                .unwrap_or(0.0);
        drop(ctx);

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
