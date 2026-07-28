// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Main trading engine event loop.
//! Drives the [`Engine`] lifecycle: subscribes to exchange streams, evaluates
//! strategy conditions via QFL runtime, manages order placement/tracking,
//! applies risk controls, and coordinates all subsystems.

use crate::indicators::{parse_using, IndicatorBank};
use crate::journal::{JournalError, JournalEvent, OrderJournal};
use crate::orders::OrderManager;
use crate::strategy_lifecycle::{DeploymentMode, StrategyLifecycle, StrategyRevision};
use crate::telemetry::RuntimeTelemetry;
use quince_core::types::*;
use quince_exchange::r#trait::{Exchange, ExchangeError, OrderRequest, StreamMsg};
use quince_logger::TradeLog;
use quince_qfl::risk::RiskLimits;
use quince_qfl::runtime::QflRuntime;
use quince_risk::RiskControls;
use sha2::{Digest, Sha256};
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
    #[error("Order journal error: {0}")]
    Journal(#[from] JournalError),
}

const ORDER_TIMEOUT: Duration = Duration::from_secs(30);
const CANCEL_RETRY_INTERVAL: Duration = Duration::from_secs(1);
const EVAL_INTERVAL: Duration = Duration::from_secs(1);
const ACCOUNT_SYNC_INTERVAL: Duration = Duration::from_secs(10);

fn strategy_artifact_digest(path: &str) -> Result<[u8; 32], EngineError> {
    let bytes = std::fs::read(path).map_err(|error| {
        EngineError::Strategy(format!("read strategy artifact {path}: {error}"))
    })?;
    Ok(Sha256::digest(bytes).into())
}

fn current_equity(
    balance_names: &[String],
    balance_values: &[f64],
    position: Option<&Position>,
) -> f64 {
    let usdt = balance_names
        .iter()
        .position(|name| name == "USDT")
        .and_then(|index| balance_values.get(index).copied())
        .unwrap_or(0.0);
    usdt + position.map(|value| value.unrealized_pnl).unwrap_or(0.0)
}

pub struct Engine<E: Exchange> {
    exchange: E,
    symbols: Vec<String>,

    orders_rx: crossbeam_channel::Receiver<Order>,

    qfl: QflRuntime,

    risk: RiskControls,
    logger: TradeLog,

    order_manager: OrderManager,
    order_journal: OrderJournal,
    // No order may cross the engine boundary until an authoritative account
    // refresh and reconciliation pass have both completed successfully.  This
    // starts false so callers which drive the engine outside `run` still fail
    // closed rather than accidentally bypassing startup preflight.
    execution_sync_ready: bool,
    execution_halted: bool,
    strategy_lifecycle: StrategyLifecycle,
    telemetry: Arc<RuntimeTelemetry>,
    indicators: IndicatorBank,

    last_price: f64,
    daily_pnl: f64,
    peak_equity: f64,
    // Account state for equity check (Vec + linear search, N в‰¤ 5)
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
        // Bind lifecycle identity to the exact input artifact, not a path or
        // placeholder. Operators can tell whether a mode transition refers to
        // the same strategy bytes.
        let artifact_digest = strategy_artifact_digest(strategy_path)?;

        // Load QFL strategy (.qfl = compile+optimize, .qfr = pre-compiled)
        let is_qfr = strategy_path.ends_with(".qfr");
        let mut qfl = if is_qfr {
            QflRuntime::load_qfr(strategy_path).map_err(EngineError::Strategy)?
        } else {
            let qfl = QflRuntime::load(strategy_path).map_err(EngineError::Strategy)?;
            let qfr_path = strategy_path.replace(".qfl", ".qfr");
            qfl.save_qfr(&qfr_path)
                .map_err(|e| EngineError::Strategy(format!("save .qfr: {}", e)))?;
            tracing::info!("optimized bytecode saved to {qfr_path}");
            qfl
        };

        // Keep the DSL-side position guard aligned with the engine's configured
        // hard position limit. The engine remains the final authority.
        qfl.set_risk_limits(RiskLimits {
            max_position: risk.max_position_size,
            ..RiskLimits::default()
        });

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

        // Finalize VM constв†’slot lookups (replaces HashMap+String in vm_getind/vm_getbal)
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
        let journal_path = std::path::Path::new(log_path).with_extension("orders.jsonl");
        let previous_records = OrderJournal::recover(&journal_path)?;
        let unresolved = OrderJournal::unresolved_client_order_ids(&previous_records);
        if !unresolved.is_empty() {
            return Err(EngineError::Strategy(format!(
                "order journal {} contains unresolved orders ({}) — reconcile them before starting a new session",
                journal_path.display(),
                unresolved.join(", ")
            )));
        }
        let order_journal = OrderJournal::open(&journal_path)?;
        let mut strategy_lifecycle = StrategyLifecycle::default();
        strategy_lifecycle
            .deploy(StrategyRevision::new(
                1,
                artifact_digest,
                DeploymentMode::Live,
            ))
            .map_err(|error| {
                EngineError::Strategy(format!("initialize strategy lifecycle: {error}"))
            })?;
        let telemetry = RuntimeTelemetry::global();
        telemetry.set_revision(
            &strategy_lifecycle
                .active()
                .expect("initial strategy revision")
                .revision,
        );

        Ok(Self {
            exchange,
            symbols: symbols.to_vec(),
            orders_rx,
            qfl,
            risk,
            logger,
            order_manager: OrderManager::new(),
            order_journal,
            execution_sync_ready: false,
            execution_halted: false,
            strategy_lifecycle,
            telemetry,
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

    /// Atomically activates a prevalidated strategy revision. Shadow revisions
    /// continue evaluation but are denied at every exchange dispatch point.
    pub fn deploy_strategy_revision(
        &mut self,
        revision: StrategyRevision,
    ) -> Result<(), EngineError> {
        self.strategy_lifecycle
            .deploy(revision)
            .map_err(|error| EngineError::Strategy(error.to_string()))?;
        self.telemetry.set_revision(
            &self
                .strategy_lifecycle
                .active()
                .expect("deployed revision")
                .revision,
        );
        Ok(())
    }

    /// Switch the active strategy between live and shadow execution without
    /// reusing its runtime state.  The mode change is represented as a new,
    /// monotonically versioned deployment so an operator can audit or roll it
    /// back through the same lifecycle boundary as a code revision.
    pub fn set_deployment_mode(&mut self, mode: DeploymentMode) -> Result<(), EngineError> {
        self.strategy_lifecycle
            .switch_mode(mode)
            .map_err(|error| EngineError::Strategy(error.to_string()))?;
        self.telemetry.set_revision(
            &self
                .strategy_lifecycle
                .active()
                .expect("mode-switched revision")
                .revision,
        );
        Ok(())
    }

    /// Promote the active shadow revision through the lifecycle boundary.
    pub fn promote_shadow_strategy(&mut self) -> Result<(), EngineError> {
        self.strategy_lifecycle
            .promote_shadow()
            .map_err(|error| EngineError::Strategy(error.to_string()))?;
        self.telemetry.set_revision(
            &self
                .strategy_lifecycle
                .active()
                .expect("promoted revision")
                .revision,
        );
        Ok(())
    }

    pub fn telemetry(&self) -> Arc<RuntimeTelemetry> {
        Arc::clone(&self.telemetry)
    }

    pub async fn run(&mut self) -> Result<(), EngineError> {
        // This is deliberately before stream subscription and, more
        // importantly, before the first order dispatch.  A stale account or
        // locally pending order is an execution-integrity failure, not a
        // recoverable warning.
        self.synchronize_execution_state().await?;
        let stream = self.exchange.subscribe(&self.symbols).await?;
        let rx = stream.rx;

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            let _ = shutdown_tx.send(());
        });

        tracing::info!(
            "engine loop starting — {} symbol(s) subscribed, {} stream(s) active",
            self.symbols.len(),
            self.symbols.len() * 2,
        );

        loop {
            if shutdown_rx.try_recv().is_ok() {
                tracing::info!("Ctrl-C received — graceful shutdown, draining VM logs");
                self.dump_vm_logs();
                return Ok(());
            }

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
                if let Err(error) = self.synchronize_execution_state().await {
                    self.execution_sync_ready = false;
                    tracing::error!(%error, "account/open-order synchronization failed; execution remains fail-closed");
                }
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
        // Keep timing entirely local to the engine event path: recording is a
        // relaxed atomic increment into a bounded histogram and cannot block
        // market-data processing.
        let started_at = Instant::now();
        self.telemetry.record_market_event();
        match msg {
            StreamMsg::Trade(trade) => {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("StreamMsg::Trade");
                self.risk.record_market_data();
                self.last_price = trade.price;
                for &(slot, v) in self.indicators.on_trade(&trade) {
                    self.qfl.set_indicator_by_slot(slot, v);
                }
                self.qfl.feed_trade(trade);
            }
            StreamMsg::Depth(depth) => {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("StreamMsg::Depth");
                self.risk.record_market_data();
                for &(slot, v) in self.indicators.on_depth(&depth) {
                    self.qfl.set_indicator_by_slot(slot, v);
                }
                self.qfl.feed_depth(depth);
            }
            StreamMsg::MarkPrice { price, .. } => {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("StreamMsg::MarkPrice");
                self.risk.record_market_data();
                self.last_price = price;
            }
            StreamMsg::OrderUpdate(fill) => {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("StreamMsg::OrderUpdate");
                let cid = self
                    .order_manager
                    .find_client_by_exchange_id(&fill.order_id);
                if let Some(cid) = cid {
                    let cid = cid.to_string();
                    let became_filled = self.order_manager.update_fill(&cid, fill.qty, fill.price);
                    if became_filled {
                        if let Err(error) = self.order_journal.append(JournalEvent::Terminal {
                            client_order_id: cid.clone(),
                            status: "FILLED".into(),
                        }) {
                            self.execution_halted = true;
                            tracing::error!(client_id = %cid, %error, "filled order was not journaled; execution halted");
                        }
                    }
                    self.logger.log_fill(&fill);
                    let mut realized_pnl = -fill.fee;
                    if let Some(pos) = self.position.as_ref() {
                        match (pos.side, fill.side) {
                            (PositionSide::Long, Side::Sell) | (PositionSide::Short, Side::Buy) => {
                                let reducing_qty = fill.qty.min(pos.size);
                                let price_diff = match pos.side {
                                    PositionSide::Long => fill.price - pos.entry_price,
                                    _ => pos.entry_price - fill.price,
                                };
                                realized_pnl += price_diff * reducing_qty;
                            }
                            _ => {}
                        }
                    }
                    self.daily_pnl += realized_pnl;
                    if realized_pnl < 0.0 {
                        self.risk.record_loss(-realized_pnl);
                    }
                    self.qfl.feed_fill(fill);
                }
            }
            StreamMsg::AccountUpdate(info) => {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("StreamMsg::AccountUpdate");
                for b in &info.balances {
                    self.set_balance(&b.asset, b.wallet);
                    self.qfl.set_balance(&b.asset, b.wallet);
                }
                let position = info
                    .positions
                    .into_iter()
                    .find(|p| p.symbol == self.symbols.first().cloned().unwrap_or_default());
                self.update_position(position);
            }
            StreamMsg::OpenInterest { .. } => {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("StreamMsg::OpenInterest");
            }
            StreamMsg::ForceOrder(_) => {
                #[cfg(feature = "profiling")]
                puffin::profile_scope!("StreamMsg::ForceOrder");
            }
            StreamMsg::ReconcileRequired { source, reason } => {
                // Private-stream gaps are an integrity boundary, not a log
                // detail. Refresh account state and query every locally
                // active order before permitting the normal loop to proceed.
                tracing::warn!(%source, %reason, "exchange reconciliation required");
                if let Err(error) = self.synchronize_execution_state().await {
                    self.execution_sync_ready = false;
                    tracing::error!(%error, "private-stream reconciliation failed; execution remains fail-closed");
                }
            }
        }
        self.telemetry
            .record_market_event_latency(started_at.elapsed());
    }

    /// Vec-based balance store — linear search over N в‰¤ 5.
    fn set_balance(&mut self, name: &str, val: f64) {
        if let Some(i) = self.balance_names.iter().position(|n| n == name) {
            self.balance_values[i] = val;
        } else {
            self.balance_names.push(name.to_string());
            self.balance_values.push(val);
        }
    }

    fn update_position(&mut self, position: Option<Position>) {
        let signed_size = position.as_ref().map_or(0.0, |pos| match pos.side {
            PositionSide::Long => pos.size,
            PositionSide::Short => -pos.size,
            PositionSide::None => 0.0,
        });
        self.position = position;
        self.qfl.set_position_size(signed_size);
    }

    /// Current account equity in the engine's quote currency.
    ///
    /// The engine currently operates USD(T)-quoted instruments, so equity is
    /// the USDT wallet balance plus the marked unrealized PnL of the tracked
    /// position. Keeping this separate from `peak_equity` is essential: the
    /// risk layer uses the latter as a high-water mark to calculate drawdown.
    fn current_equity(&self) -> f64 {
        current_equity(
            &self.balance_names,
            &self.balance_values,
            self.position.as_ref(),
        )
    }

    async fn on_strategy_order(&mut self, order: Order) {
        self.telemetry.record_order_intent();
        if !self.strategy_lifecycle.execution_enabled() {
            self.telemetry.record_suppressed_order();
            tracing::info!(symbol = %order.symbol, "shadow strategy order suppressed before journal/exchange");
            return;
        }
        if self.execution_halted {
            tracing::error!("refusing order: execution is halted after an order-journal failure");
            return;
        }
        if !self.execution_sync_ready {
            tracing::error!(symbol = %order.symbol, "refusing order: mandatory account/open-order synchronization has not completed");
            return;
        }
        let current_position = self.position.as_ref().map_or(0.0, |pos| match pos.side {
            PositionSide::Long => pos.size,
            PositionSide::Short => -pos.size,
            PositionSide::None => 0.0,
        }) + self.order_manager.pending_signed_exposure();
        // Pass the account's current marked equity, not its historical high-water
        // mark. `RiskControls` owns the high-water mark and pauses execution when
        // the real drawdown breaches the configured limit.
        let current_equity = self.current_equity();
        if let Err(reason) = self
            .risk
            .check_order(&order, current_equity, current_position)
        {
            tracing::warn!("risk rejected order: {}", reason);
            return;
        }

        let client_id = self.order_manager.register(order);
        let Some(pending) = self.order_manager.get(&client_id) else {
            tracing::error!(
                client_id,
                "freshly registered order disappeared before journaling"
            );
            return;
        };
        if let Err(error) = self.order_journal.append(JournalEvent::Registered {
            client_order_id: client_id.clone(),
            symbol: pending.order.symbol.to_string(),
            side: format!("{:?}", pending.order.side).to_ascii_lowercase(),
            qty: pending.order.qty,
            reduce_only: pending.order.reduce_only,
        }) {
            self.order_manager
                .mark_failed(&client_id, error.to_string());
            self.execution_halted = true;
            tracing::error!(client_id, %error, "refusing order because its journal registration was not durable; execution halted");
            return;
        }
        if let Some(po) = self.order_manager.get(&client_id) {
            let request = OrderRequest {
                client_order_id: client_id.clone(),
                order: po.order.clone(),
            };
            match self.exchange.place_order(request).await {
                Ok(order_id) => {
                    self.order_manager.mark_placed(&client_id, order_id);
                    if let Some(order_id) = self.order_manager.exchange_order_id(&client_id) {
                        if let Err(error) = self.order_journal.append(JournalEvent::Accepted {
                            client_order_id: client_id.clone(),
                            exchange_order_id: order_id.to_string(),
                        }) {
                            self.execution_halted = true;
                            tracing::error!(client_id, %error, "accepted order was not journaled; execution halted");
                        }
                    }
                    self.risk.record_trade();
                }
                Err(e) => {
                    // Only explicit exchange-side validation/authentication
                    // failures are safe to treat as terminal.  A transport
                    // failure may have happened after the exchange accepted
                    // the request, so keep its possible exposure visible.
                    match e {
                        ExchangeError::Order(_) | ExchangeError::Auth(_) => {
                            self.order_manager.mark_failed(&client_id, e.to_string());
                            if let Err(error) = self.order_journal.append(JournalEvent::Terminal {
                                client_order_id: client_id.clone(),
                                status: "REJECTED".into(),
                            }) {
                                self.execution_halted = true;
                                tracing::error!(client_id, %error, "explicitly rejected order was not journaled; execution halted");
                            }
                            tracing::warn!(client_id, error = %e, "exchange explicitly rejected order");
                        }
                        ExchangeError::Ws(_)
                        | ExchangeError::Rest(_)
                        | ExchangeError::Timeout
                        | ExchangeError::Disconnected => {
                            self.order_manager
                                .mark_submission_unknown(&client_id, e.to_string());
                            if let Err(error) =
                                self.order_journal.append(JournalEvent::SubmissionUnknown {
                                    client_order_id: client_id.clone(),
                                    error: e.to_string(),
                                })
                            {
                                self.execution_halted = true;
                                tracing::error!(client_id, %error, "unknown submission was not journaled; execution halted");
                            }
                            tracing::error!(
                                client_id,
                                error = %e,
                                "order submission outcome is unknown; refusing automatic retry"
                            );
                        }
                    }
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
            let signed_size = match pos.side {
                PositionSide::Long => pos.size,
                PositionSide::Short => -pos.size,
                PositionSide::None => 0.0,
            };
            self.qfl.set_position_size(signed_size);
            self.qfl
                .set_indicator_by_slot(self.entry_price_slot, pos.entry_price);
            self.qfl
                .set_indicator_by_slot(self.unrealized_pnl_slot, pos.unrealized_pnl);
        } else {
            self.qfl.set_position_size(0.0);
            self.qfl.set_indicator_by_slot(self.entry_price_slot, 0.0);
            self.qfl
                .set_indicator_by_slot(self.unrealized_pnl_slot, 0.0);
        }
        self.qfl.feed_eval();
        self.equity_check();
    }

    async fn sync_account(&mut self) -> Result<(), EngineError> {
        #[cfg(feature = "profiling")]
        puffin::profile_scope!("sync_account");
        let info = self.exchange.account_info().await?;
        for b in &info.balances {
            self.set_balance(&b.asset, b.wallet);
            self.qfl.set_balance(&b.asset, b.wallet);
        }
        let position = info
            .positions
            .into_iter()
            .find(|p| p.symbol == self.symbols.first().cloned().unwrap_or_default());
        self.update_position(position);
        Ok(())
    }

    /// Refresh the authoritative account view and reconcile every locally
    /// active order.  The order manager is intentionally the reconciliation
    /// scope here: an engine may only own orders it journaled, while foreign
    /// exchange orders must never be silently adopted into strategy state.
    async fn synchronize_execution_state(&mut self) -> Result<(), EngineError> {
        self.execution_sync_ready = false;
        self.sync_account().await?;
        self.reconcile_pending_orders().await?;
        self.execution_sync_ready = true;
        Ok(())
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
                        && now.duration_since(po.last_update) > CANCEL_RETRY_INTERVAL
                } else {
                    false
                }
            })
            .collect();

        for cid in timed_out {
            let Some(po) = self.order_manager.get(&cid) else {
                continue;
            };
            let symbol = po.order.symbol.to_string();
            let order_id = match self
                .order_manager
                .exchange_order_id(&cid)
                .map(str::to_owned)
            {
                Some(order_id) => order_id,
                None => match self.exchange.order_status_by_client_id(&symbol, &cid).await {
                    Ok(status) => {
                        if let Err(error) = self.order_manager.reconcile_client_order(
                            &cid,
                            &status.order_id,
                            &status.status,
                            status.filled_qty,
                            status.avg_price,
                        ) {
                            tracing::warn!(client_id = %cid, %error, "client-order-id reconciliation failed");
                            continue;
                        }
                        match self.order_manager.exchange_order_id(&cid) {
                            Some(order_id) => order_id.to_owned(),
                            None => {
                                if let Err(error) =
                                    self.order_journal.append(JournalEvent::Terminal {
                                        client_order_id: cid.clone(),
                                        status: status.status,
                                    })
                                {
                                    self.execution_halted = true;
                                    tracing::error!(client_id = %cid, %error, "reconciled terminal order was not journaled; execution halted");
                                }
                                continue;
                            }
                        }
                    }
                    Err(error) => {
                        // The order remains risk-visible. Adapters without a
                        // native client-ID lookup must be reconciled manually.
                        tracing::error!(client_id = %cid, %error, "unresolved order submission requires client-id reconciliation");
                        continue;
                    }
                },
            };

            match self.exchange.order_status(&symbol, &order_id).await {
                Ok(status) => {
                    if let Err(error) = self.order_manager.reconcile_status(
                        &cid,
                        &status.status,
                        status.filled_qty,
                        status.avg_price,
                    ) {
                        tracing::warn!(client_id = %cid, order_id = %order_id, %error, "invalid order status reconciliation");
                        continue;
                    }
                    if self.order_manager.exchange_order_id(&cid).is_none() {
                        if let Err(error) = self.order_journal.append(JournalEvent::Terminal {
                            client_order_id: cid.clone(),
                            status: status.status.clone(),
                        }) {
                            self.execution_halted = true;
                            tracing::error!(client_id = %cid, %error, "terminal order was not journaled; execution halted");
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(client_id = %cid, order_id = %order_id, %error, "order status reconciliation failed");
                    continue;
                }
            }

            // Status could have transitioned to a terminal value.
            let Some(order_id) = self
                .order_manager
                .exchange_order_id(&cid)
                .map(str::to_owned)
            else {
                continue;
            };
            match self.exchange.cancel_order(&symbol, &order_id).await {
                Ok(()) => {
                    self.order_manager.mark_cancel_requested(&cid);
                    if let Err(error) = self.order_journal.append(JournalEvent::CancelRequested {
                        client_order_id: cid.clone(),
                        exchange_order_id: order_id.clone(),
                    }) {
                        self.execution_halted = true;
                        tracing::error!(client_id = %cid, %error, "cancel request was not journaled; execution halted");
                    }
                    tracing::warn!(client_id = %cid, order_id = %order_id, "order timeout: cancellation requested, awaiting confirmation");
                }
                Err(error) => {
                    tracing::warn!(client_id = %cid, order_id = %order_id, %error, "timed-out order cancellation failed; keeping exposure active")
                }
            }
        }

        self.order_manager.cleanup_terminal();
    }

    /// Authoritative, non-mutating reconciliation used after a private-stream
    /// gap. Timeout handling owns cancellation; this path only observes state.
    async fn reconcile_pending_orders(&mut self) -> Result<(), EngineError> {
        for cid in self.order_manager.pending_order_ids() {
            let Some(po) = self.order_manager.get(&cid) else {
                continue;
            };
            let symbol = po.order.symbol.to_string();
            let status = match self
                .order_manager
                .exchange_order_id(&cid)
                .map(str::to_owned)
            {
                Some(order_id) => self.exchange.order_status(&symbol, &order_id).await,
                None => self.exchange.order_status_by_client_id(&symbol, &cid).await,
            };
            let status = match status {
                Ok(status) => status,
                Err(error) => {
                    tracing::error!(client_id = %cid, %error, "immediate order reconciliation failed; exposure remains active");
                    return Err(error.into());
                }
            };
            let result = if self.order_manager.exchange_order_id(&cid).is_some() {
                self.order_manager.reconcile_status(
                    &cid,
                    &status.status,
                    status.filled_qty,
                    status.avg_price,
                )
            } else {
                self.order_manager.reconcile_client_order(
                    &cid,
                    &status.order_id,
                    &status.status,
                    status.filled_qty,
                    status.avg_price,
                )
            };
            if let Err(error) = result {
                tracing::error!(client_id = %cid, %error, "invalid immediate order reconciliation");
                return Err(EngineError::Strategy(format!(
                    "invalid authoritative order reconciliation for {cid}: {error}"
                )));
            }
            if matches!(
                status.status.trim().to_ascii_uppercase().as_str(),
                "FILLED" | "CANCELED" | "CANCELLED" | "EXPIRED" | "REJECTED"
            ) {
                if let Err(error) = self.order_journal.append(JournalEvent::Terminal {
                    client_order_id: cid.clone(),
                    status: status.status,
                }) {
                    self.execution_halted = true;
                    tracing::error!(client_id = %cid, %error, "terminal reconciliation was not journaled; execution halted");
                    return Err(error.into());
                }
            }
        }
        self.order_manager.cleanup_terminal();
        Ok(())
    }

    async fn check_sl_tp(&mut self) {
        if !self.strategy_lifecycle.execution_enabled() {
            return;
        }
        let price = self.last_price;
        if !price.is_finite() || price <= 0.0 {
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
                symbol: self
                    .symbols
                    .first()
                    .map(|s| Arc::<str>::from(s.as_str()))
                    .unwrap_or_else(|| Arc::from("")),
                side,
                qty,
                price: None,
                order_type: OrderType::Market,
                reduce_only: true,
                stop_loss: None,
                take_profit: None,
            };
            let close_client_id = self.order_manager.register(close);
            let Some(close_pending) = self.order_manager.get(&close_client_id) else {
                tracing::error!(client_id = %close_client_id, "freshly registered stop-loss/take-profit close order disappeared");
                continue;
            };
            if let Err(error) = self.order_journal.append(JournalEvent::Registered {
                client_order_id: close_client_id.clone(),
                symbol: close_pending.order.symbol.to_string(),
                side: format!("{:?}", close_pending.order.side).to_ascii_lowercase(),
                qty: close_pending.order.qty,
                reduce_only: true,
            }) {
                self.order_manager
                    .mark_failed(&close_client_id, error.to_string());
                self.execution_halted = true;
                tracing::error!(client_id = %close_client_id, %error, "refusing protective close because journal registration failed; execution halted");
                continue;
            }
            let request = self
                .order_manager
                .get(&close_client_id)
                .map(|pending| OrderRequest {
                    client_order_id: close_client_id.clone(),
                    order: pending.order.clone(),
                })
                .expect("freshly registered close order must exist");
            match self.exchange.place_order(request).await {
                Ok(id) => {
                    self.order_manager.mark_placed(&close_client_id, id.clone());
                    if let Err(error) = self.order_journal.append(JournalEvent::Accepted {
                        client_order_id: close_client_id.clone(),
                        exchange_order_id: id.clone(),
                    }) {
                        self.execution_halted = true;
                        tracing::error!(client_id = %close_client_id, %error, "accepted protective close was not journaled; execution halted");
                    }
                    self.order_manager.deactivate_sl_tp(&cid);
                    tracing::info!("SL/TP triggered for {cid}: close {side:?} {qty} order={id}");
                }
                Err(e) => {
                    match e {
                        // Explicit exchange-side rejections are terminal and
                        // must close the durable lifecycle as well. Leaving a
                        // registered close order unresolved would block a
                        // safe restart even though no remote order exists.
                        ExchangeError::Order(_) | ExchangeError::Auth(_) => {
                            self.order_manager
                                .mark_failed(&close_client_id, e.to_string());
                            if let Err(error) = self.order_journal.append(JournalEvent::Terminal {
                                client_order_id: close_client_id.clone(),
                                status: "REJECTED".into(),
                            }) {
                                self.execution_halted = true;
                                tracing::error!(client_id = %close_client_id, %error, "rejected protective close was not journaled; execution halted");
                            }
                            tracing::warn!(client_id = %close_client_id, error = %e, "exchange explicitly rejected protective close");
                        }
                        ExchangeError::Ws(_)
                        | ExchangeError::Rest(_)
                        | ExchangeError::Timeout
                        | ExchangeError::Disconnected => {
                            self.order_manager
                                .mark_submission_unknown(&close_client_id, e.to_string());
                            if let Err(error) =
                                self.order_journal.append(JournalEvent::SubmissionUnknown {
                                    client_order_id: close_client_id.clone(),
                                    error: e.to_string(),
                                })
                            {
                                self.execution_halted = true;
                                tracing::error!(client_id = %close_client_id, %error, "unknown protective close was not journaled; execution halted");
                            }
                            tracing::warn!(client_id = %close_client_id, error = %e, "protective close submission outcome is unknown");
                        }
                    }
                }
            }
        }
    }

    fn equity_check(&mut self) {
        let equity = self.current_equity();

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

    // Drain the VM log ring buffer into qflvm.log (debug builds only).
    pub fn dump_vm_logs(&mut self) {
        let logs = self.qfl.dump_vm_logs();
        if !logs.is_empty() {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("qflvm.log")
            {
                use std::io::Write;
                for log in &logs {
                    let _ = writeln!(file, "{log}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StartupSyncExchange {
        account_error: bool,
    }

    #[async_trait::async_trait]
    impl Exchange for StartupSyncExchange {
        async fn subscribe(
            &self,
            _symbols: &[String],
        ) -> quince_exchange::r#trait::Result<quince_exchange::r#trait::Stream> {
            let (_tx, rx) = crossbeam_channel::bounded(1);
            Ok(quince_exchange::r#trait::Stream { rx })
        }

        async fn place_order(
            &self,
            _request: OrderRequest,
        ) -> quince_exchange::r#trait::Result<String> {
            Ok("test-order".into())
        }

        async fn cancel_order(
            &self,
            _symbol: &str,
            _order_id: &str,
        ) -> quince_exchange::r#trait::Result<()> {
            Ok(())
        }

        async fn order_status(
            &self,
            symbol: &str,
            order_id: &str,
        ) -> quince_exchange::r#trait::Result<quince_exchange::r#trait::OrderStatus> {
            Ok(quince_exchange::r#trait::OrderStatus {
                order_id: order_id.into(),
                symbol: symbol.into(),
                side: Side::Buy,
                qty: 0.0,
                filled_qty: 0.0,
                price: 0.0,
                avg_price: 0.0,
                status: "NEW".into(),
            })
        }

        async fn account_info(&self) -> quince_exchange::r#trait::Result<AccountInfo> {
            if self.account_error {
                return Err(ExchangeError::Rest("account unavailable".into()));
            }
            Ok(AccountInfo {
                balances: Vec::new(),
                positions: Vec::new(),
            })
        }

        async fn current_price(&self, _symbol: &str) -> quince_exchange::r#trait::Result<f64> {
            Ok(100.0)
        }
    }

    fn startup_sync_engine(
        account_error: bool,
    ) -> (Engine<StartupSyncExchange>, std::path::PathBuf) {
        let path =
            std::env::temp_dir().join(format!("quince-startup-sync-{}.qfl", uuid::Uuid::new_v4()));
        std::fs::write(&path, "function on_eval()\nend\n").unwrap();
        let log_path = path.with_extension("trades.jsonl");
        let engine = Engine::new(
            StartupSyncExchange { account_error },
            &["BTCUSDT".into()],
            path.to_str().unwrap(),
            RiskControls::new(quince_risk::RiskConfig::default()),
            log_path.to_str().unwrap(),
        )
        .unwrap();
        (engine, path)
    }

    fn remove_startup_sync_artifacts(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("qfr"));
        let _ = std::fs::remove_file(path.with_extension("trades.jsonl"));
        let _ = std::fs::remove_file(path.with_extension("orders.jsonl"));
    }

    fn test_order() -> Order {
        Order {
            symbol: Arc::from("BTCUSDT"),
            side: Side::Buy,
            qty: 0.1,
            price: None,
            order_type: OrderType::Market,
            reduce_only: false,
            stop_loss: None,
            take_profit: None,
        }
    }

    #[tokio::test]
    async fn startup_sync_allows_empty_read_only_account() {
        let (mut engine, path) = startup_sync_engine(false);
        assert!(!engine.execution_sync_ready);
        engine.synchronize_execution_state().await.unwrap();
        assert!(engine.execution_sync_ready);
        remove_startup_sync_artifacts(&path);
    }

    #[tokio::test]
    async fn startup_sync_failure_keeps_execution_fail_closed() {
        let (mut engine, path) = startup_sync_engine(true);
        assert!(engine.synchronize_execution_state().await.is_err());
        assert!(!engine.execution_sync_ready);
        remove_startup_sync_artifacts(&path);
    }

    #[test]
    fn current_equity_includes_marked_unrealized_pnl() {
        let names = vec!["BTC".to_string(), "USDT".to_string()];
        let values = vec![0.5, 10_000.0];
        let position = Position {
            symbol: "BTCUSDT".to_string(),
            side: PositionSide::Long,
            size: 0.1,
            entry_price: 100_000.0,
            unrealized_pnl: -750.0,
        };

        assert_eq!(current_equity(&names, &values, Some(&position)), 9_250.0);
    }

    #[test]
    fn current_marked_equity_trips_and_latches_drawdown_gate() {
        let config = quince_risk::RiskConfig {
            max_drawdown: 0.10,
            ..quince_risk::RiskConfig::default()
        };
        let mut risk = RiskControls::new(config);
        risk.record_market_data();
        risk.check_order(&test_order(), 10_000.0, 0.0)
            .expect("initial equity should establish the high-water mark");

        let equity = current_equity(
            &["USDT".to_string()],
            &[9_000.0],
            Some(&Position {
                symbol: "BTCUSDT".to_string(),
                side: PositionSide::Long,
                size: 0.1,
                entry_price: 100_000.0,
                unrealized_pnl: -1_100.0,
            }),
        );
        let error = risk
            .check_order(&test_order(), equity, 0.0)
            .expect_err("real marked equity is below the drawdown limit");

        assert!(error.contains("drawdown"));
        assert!(risk.is_paused());
    }

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

    #[test]
    fn strategy_digest_is_stable_and_content_addressed() {
        let path = std::env::temp_dir().join(format!("quince-digest-{}.qfl", uuid::Uuid::new_v4()));
        std::fs::write(&path, "on trade(t) {}\n").unwrap();
        let path = path.to_str().unwrap();

        let first = strategy_artifact_digest(path).unwrap();
        let second = strategy_artifact_digest(path).unwrap();
        assert_eq!(first, second);
        assert_ne!(first, [0; 32]);

        std::fs::remove_file(path).unwrap();
    }
}
