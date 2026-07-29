// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Deterministic, offline QFL market-data replay.
//!
//! The input is newline-delimited JSON. Every line has `schema_version: 1`
//! and one of these event shapes:
//! `{"schema_version":1,"type":"trade","timestamp_ms":...,"price":...,
//!   "qty":...,"side":"buy|sell","trade_id":...}`;
//! `{"schema_version":1,"type":"depth","timestamp_ms":...,"bids":[{"price":...,"qty":...}],
//!   "asks":[...]}`; or `{"schema_version":1,"type":"eval","timestamp_ms":...}`.
//!
//! Replay never opens a socket and never sends an exchange order.  QFL order
//! intents are captured in-memory and reported as deterministic counters.

use chrono::{TimeZone, Utc};
use quince::core::types::{Depth, DepthLevel, Order, OrderFill, OrderType, Side, Trade};
use quince::qfl::runtime::QflRuntime;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, BufReader};

const SCHEMA_VERSION: u8 = 1;
const DEFAULT_INITIAL_EQUITY_QUOTE: f64 = 10_000.0;

#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("read replay file {path}: {source}")]
    Open {
        path: String,
        source: std::io::Error,
    },
    #[error("read replay line {line}: {source}")]
    Read { line: usize, source: std::io::Error },
    #[error("invalid replay line {line}: {reason}")]
    Invalid { line: usize, reason: String },
    #[error("load strategy: {0}")]
    Strategy(String),
    #[error("invalid replay cost model: {0}")]
    CostModel(String),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ReplayEvent {
    Trade {
        schema_version: u8,
        timestamp_ms: i64,
        price: f64,
        qty: f64,
        side: ReplaySide,
        trade_id: u64,
    },
    Depth {
        schema_version: u8,
        timestamp_ms: i64,
        bids: Vec<ReplayLevel>,
        asks: Vec<ReplayLevel>,
    },
    Eval {
        schema_version: u8,
        timestamp_ms: i64,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReplaySide {
    Buy,
    Sell,
}

impl From<ReplaySide> for Side {
    fn from(value: ReplaySide) -> Self {
        match value {
            ReplaySide::Buy => Self::Buy,
            ReplaySide::Sell => Self::Sell,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ReplayLevel {
    price: f64,
    qty: f64,
}

/// Taker-style cost assumptions for offline paper execution.
///
/// The defaults are intentionally conservative: 10 bps fee and 5 bps of
/// adverse slippage per fill.  They are not an exchange fee schedule.  Set
/// `QUINCE_REPLAY_FEE_BPS` and `QUINCE_REPLAY_SLIPPAGE_BPS` to model a
/// specific venue/account tier.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ReplayCostModel {
    pub fee_bps: f64,
    pub slippage_bps: f64,
}

impl Default for ReplayCostModel {
    fn default() -> Self {
        Self {
            fee_bps: 10.0,
            slippage_bps: 5.0,
        }
    }
}

impl ReplayCostModel {
    fn from_env() -> Result<Self, ReplayError> {
        let defaults = Self::default();
        let fee_bps = replay_bps_env("QUINCE_REPLAY_FEE_BPS", defaults.fee_bps)?;
        let slippage_bps = replay_bps_env("QUINCE_REPLAY_SLIPPAGE_BPS", defaults.slippage_bps)?;
        Self {
            fee_bps,
            slippage_bps,
        }
        .validate()
    }

    fn validate(self) -> Result<Self, ReplayError> {
        for (name, value) in [
            ("fee_bps", self.fee_bps),
            ("slippage_bps", self.slippage_bps),
        ] {
            if !value.is_finite() || value < 0.0 || value > 10_000.0 {
                return Err(ReplayError::CostModel(format!(
                    "{name} must be finite and between 0 and 10000"
                )));
            }
        }
        Ok(self)
    }
}

fn replay_bps_env(name: &str, default: f64) -> Result<f64, ReplayError> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<f64>()
            .map_err(|_| ReplayError::CostModel(format!("{name} must be a finite number"))),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(ReplayError::CostModel(format!("read {name}: {error}"))),
    }
}

fn replay_initial_equity_from_env() -> Result<f64, ReplayError> {
    let value = match std::env::var("QUINCE_REPLAY_INITIAL_EQUITY") {
        Ok(value) => value.parse::<f64>().map_err(|_| {
            ReplayError::CostModel("QUINCE_REPLAY_INITIAL_EQUITY must be a finite number".into())
        })?,
        Err(std::env::VarError::NotPresent) => DEFAULT_INITIAL_EQUITY_QUOTE,
        Err(error) => {
            return Err(ReplayError::CostModel(format!(
                "read QUINCE_REPLAY_INITIAL_EQUITY: {error}"
            )))
        }
    };
    if !value.is_finite() || value <= 0.0 {
        return Err(ReplayError::CostModel(
            "QUINCE_REPLAY_INITIAL_EQUITY must be finite and greater than zero".into(),
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplaySummary {
    pub schema_version: u8,
    pub events: u64,
    pub trades: u64,
    pub depth_snapshots: u64,
    pub eval_ticks: u64,
    /// QFL `quince.order` outputs. They are intents only: no exchange path is
    /// instantiated during replay.
    pub order_intents: u64,
    pub buy_intents: u64,
    pub sell_intents: u64,
    /// Strategy-authored `quince.log` entries captured during this offline run.
    pub strategy_logs: u64,
    /// Log entries containing `signal`; useful for signal-only strategies.
    pub signal_logs: u64,
    /// First strategy log messages, bounded so a noisy strategy cannot turn a
    /// replay report into an unbounded allocation.
    pub log_samples: Vec<String>,
    /// The paper-execution assumptions used for this run. Replay never sends
    /// these fills to an exchange.
    pub cost_model: ReplayCostModel,
    /// Immediately marketable QFL order intents filled against the latest
    /// captured top-of-book. Resting/non-marketable limits remain unfilled.
    pub paper_fills: u64,
    pub unfilled_intents: u64,
    pub filled_notional_quote: f64,
    pub fees_quote: f64,
    pub slippage_cost_quote: f64,
    pub realized_gross_pnl_quote: f64,
    pub unrealized_gross_pnl_quote: f64,
    pub gross_pnl_quote: f64,
    pub net_pnl_quote: f64,
    pub ending_position_qty: f64,
    pub ending_mark_price: Option<f64>,
    /// Mark-to-market statistics sampled after each replay event. Ratios are
    /// per observation, deliberately not annualized from irregular trade data.
    pub performance: ReplayPerformance,
}

/// Reproducible performance statistics for one offline replay.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplayPerformance {
    pub initial_equity_quote: f64,
    pub ending_equity_quote: f64,
    pub net_return_fraction: f64,
    pub max_drawdown_fraction: f64,
    pub observations: u64,
    pub mean_return_per_observation: f64,
    pub volatility_per_observation: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sharpe_per_observation: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sortino_per_observation: Option<f64>,
}

#[derive(Debug)]
struct EquityTracker {
    initial_equity_quote: f64,
    previous_equity_quote: f64,
    peak_equity_quote: f64,
    observations: u64,
    return_count: u64,
    return_sum: f64,
    return_sum_sq: f64,
    downside_return_sq: f64,
    downside_count: u64,
    max_drawdown_fraction: f64,
}

impl EquityTracker {
    fn new(initial_equity_quote: f64) -> Self {
        Self {
            initial_equity_quote,
            previous_equity_quote: initial_equity_quote,
            peak_equity_quote: initial_equity_quote,
            observations: 0,
            return_count: 0,
            return_sum: 0.0,
            return_sum_sq: 0.0,
            downside_return_sq: 0.0,
            downside_count: 0,
            max_drawdown_fraction: 0.0,
        }
    }

    fn observe(&mut self, equity_quote: f64) {
        if !equity_quote.is_finite() {
            return;
        }
        self.observations += 1;
        if self.previous_equity_quote > 0.0 {
            let period_return = equity_quote / self.previous_equity_quote - 1.0;
            if period_return.is_finite() {
                self.return_count += 1;
                self.return_sum += period_return;
                self.return_sum_sq += period_return * period_return;
                if period_return < 0.0 {
                    self.downside_count += 1;
                    self.downside_return_sq += period_return * period_return;
                }
            }
        }
        self.peak_equity_quote = self.peak_equity_quote.max(equity_quote);
        if self.peak_equity_quote > 0.0 {
            self.max_drawdown_fraction = self
                .max_drawdown_fraction
                .max((self.peak_equity_quote - equity_quote) / self.peak_equity_quote);
        }
        self.previous_equity_quote = equity_quote;
    }

    fn finish(self, ending_equity_quote: f64) -> ReplayPerformance {
        let mean = if self.return_count > 0 {
            self.return_sum / self.return_count as f64
        } else {
            0.0
        };
        let variance = (self.return_count > 1).then(|| {
            (self.return_sum_sq - self.return_count as f64 * mean * mean)
                / (self.return_count - 1) as f64
        });
        let volatility = variance.map(|value| value.max(0.0).sqrt()).unwrap_or(0.0);
        let downside_deviation = (self.downside_count > 0)
            .then(|| (self.downside_return_sq / self.downside_count as f64).sqrt());
        ReplayPerformance {
            initial_equity_quote: self.initial_equity_quote,
            ending_equity_quote,
            net_return_fraction: (ending_equity_quote - self.initial_equity_quote)
                / self.initial_equity_quote,
            max_drawdown_fraction: self.max_drawdown_fraction,
            observations: self.observations,
            mean_return_per_observation: mean,
            volatility_per_observation: volatility,
            sharpe_per_observation: (volatility > 0.0).then(|| mean / volatility),
            sortino_per_observation: downside_deviation
                .filter(|value| *value > 0.0)
                .map(|value| mean / value),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ReplayBook {
    best_bid: Option<f64>,
    best_ask: Option<f64>,
    last_trade: Option<f64>,
}

impl ReplayBook {
    fn mark_price(self) -> Option<f64> {
        match (self.best_bid, self.best_ask) {
            (Some(bid), Some(ask)) => Some((bid + ask) / 2.0),
            _ => self.last_trade,
        }
    }

    fn reference_price(self, side: Side) -> Option<f64> {
        match side {
            Side::Buy => self.best_ask.or(self.last_trade),
            Side::Sell => self.best_bid.or(self.last_trade),
        }
    }
}

#[derive(Debug, Default)]
struct PaperPortfolio {
    position_qty: f64,
    average_entry_price: f64,
    realized_gross_pnl_quote: f64,
}

impl PaperPortfolio {
    fn apply_fill(&mut self, side: Side, qty: f64, price: f64) {
        let delta = if side == Side::Buy { qty } else { -qty };
        if self.position_qty == 0.0 || self.position_qty.signum() == delta.signum() {
            let existing_notional = self.position_qty.abs() * self.average_entry_price;
            self.position_qty += delta;
            self.average_entry_price = (existing_notional + qty * price) / self.position_qty.abs();
            return;
        }

        let closing_qty = self.position_qty.abs().min(qty);
        self.realized_gross_pnl_quote += if self.position_qty > 0.0 {
            (price - self.average_entry_price) * closing_qty
        } else {
            (self.average_entry_price - price) * closing_qty
        };
        let remaining = qty - closing_qty;
        self.position_qty += delta;
        if self.position_qty == 0.0 {
            self.average_entry_price = 0.0;
        } else if remaining > 0.0 {
            self.average_entry_price = price;
        }
    }

    fn unrealized_gross_pnl(&self, mark_price: Option<f64>) -> f64 {
        let Some(mark) = mark_price else {
            return 0.0;
        };
        if self.position_qty > 0.0 {
            (mark - self.average_entry_price) * self.position_qty
        } else {
            (self.average_entry_price - mark) * -self.position_qty
        }
    }
}

impl ReplaySummary {
    fn record_orders(
        &mut self,
        runtime: &mut QflRuntime,
        orders: impl Iterator<Item = Order>,
        book: ReplayBook,
        portfolio: &mut PaperPortfolio,
        fill_time: chrono::DateTime<Utc>,
    ) {
        for order in orders {
            self.order_intents += 1;
            match order.side {
                Side::Buy => self.buy_intents += 1,
                Side::Sell => self.sell_intents += 1,
            }
            let Some(reference_price) = book.reference_price(order.side) else {
                self.unfilled_intents += 1;
                continue;
            };
            if !is_marketable(&order, reference_price) {
                self.unfilled_intents += 1;
                continue;
            }
            let fill_price = adverse_fill_price(
                &order,
                reference_price,
                self.cost_model.slippage_bps / 10_000.0,
            );
            let notional = fill_price * order.qty;
            let fee = notional * self.cost_model.fee_bps / 10_000.0;
            self.paper_fills += 1;
            self.filled_notional_quote += notional;
            self.fees_quote += fee;
            self.slippage_cost_quote += (fill_price - reference_price).abs() * order.qty;
            portfolio.apply_fill(order.side, order.qty, fill_price);
            runtime.feed_fill(OrderFill {
                order_id: format!("replay-{}", self.paper_fills),
                side: order.side,
                price: fill_price,
                qty: order.qty,
                fee,
                fee_asset: "QUOTE".into(),
                time: fill_time,
            });
        }
    }

    fn finalize_pnl(&mut self, portfolio: &PaperPortfolio, mark_price: Option<f64>) {
        self.ending_position_qty = portfolio.position_qty;
        self.ending_mark_price = mark_price;
        self.realized_gross_pnl_quote = portfolio.realized_gross_pnl_quote;
        self.unrealized_gross_pnl_quote = portfolio.unrealized_gross_pnl(mark_price);
        self.gross_pnl_quote = self.realized_gross_pnl_quote + self.unrealized_gross_pnl_quote;
        self.net_pnl_quote = self.gross_pnl_quote - self.fees_quote;
    }
}

fn is_marketable(order: &Order, reference_price: f64) -> bool {
    match (order.order_type, order.price) {
        (OrderType::Market, _) | (_, None) => true,
        (_, Some(limit)) if order.side == Side::Buy => limit >= reference_price,
        (_, Some(limit)) => limit <= reference_price,
    }
}

/// Apply adverse slippage while preserving a marketable limit's price bound.
///
/// A buy limit may never fill above its limit, and a sell limit may never fill
/// below it.  At-touch limits therefore fill at the touch rather than being
/// incorrectly worsened through their own limit.
fn adverse_fill_price(order: &Order, reference_price: f64, slippage: f64) -> f64 {
    let adverse_price = match order.side {
        Side::Buy => reference_price * (1.0 + slippage),
        Side::Sell => reference_price * (1.0 - slippage),
    };

    match (order.order_type, order.price) {
        (OrderType::Limit, Some(limit)) if order.side == Side::Buy => adverse_price.min(limit),
        (OrderType::Limit, Some(limit)) => adverse_price.max(limit),
        _ => adverse_price,
    }
}

/// Replay a versioned JSONL market-data capture through a QFL strategy.
///
/// Event order is the file order, deliberately: no wall-clock scheduling,
/// random identifiers, exchange requests, or parallel dispatch are involved.
pub fn run(
    strategy_path: &str,
    replay_path: &str,
    symbol: &str,
) -> Result<ReplaySummary, ReplayError> {
    run_with_settings(
        strategy_path,
        replay_path,
        symbol,
        ReplayCostModel::from_env()?,
        replay_initial_equity_from_env()?,
    )
}

/// As [`run`], with explicit cost assumptions for deterministic tests and
/// programmatic callers. It is still strictly offline paper execution.
#[cfg(test)]
pub fn run_with_cost_model(
    strategy_path: &str,
    replay_path: &str,
    symbol: &str,
    cost_model: ReplayCostModel,
) -> Result<ReplaySummary, ReplayError> {
    run_with_settings(
        strategy_path,
        replay_path,
        symbol,
        cost_model,
        DEFAULT_INITIAL_EQUITY_QUOTE,
    )
}

fn run_with_settings(
    strategy_path: &str,
    replay_path: &str,
    symbol: &str,
    cost_model: ReplayCostModel,
    initial_equity_quote: f64,
) -> Result<ReplaySummary, ReplayError> {
    let cost_model = cost_model.validate()?;
    if !initial_equity_quote.is_finite() || initial_equity_quote <= 0.0 {
        return Err(ReplayError::CostModel(
            "initial equity must be finite and greater than zero".into(),
        ));
    }
    if symbol.trim().is_empty() {
        return Err(ReplayError::Invalid {
            line: 0,
            reason: "symbol must not be empty".into(),
        });
    }
    let mut runtime = if strategy_path.ends_with(".qfr") {
        QflRuntime::load_qfr(strategy_path).map_err(ReplayError::Strategy)?
    } else {
        QflRuntime::load(strategy_path).map_err(ReplayError::Strategy)?
    };
    let (orders_tx, orders_rx) = crossbeam_channel::unbounded();
    runtime.set_order_sender(orders_tx);
    runtime.set_symbol(symbol);
    runtime.finalize_vm_init();

    let file = File::open(replay_path).map_err(|source| ReplayError::Open {
        path: replay_path.into(),
        source,
    })?;
    let mut summary = ReplaySummary {
        schema_version: SCHEMA_VERSION,
        events: 0,
        trades: 0,
        depth_snapshots: 0,
        eval_ticks: 0,
        order_intents: 0,
        buy_intents: 0,
        sell_intents: 0,
        strategy_logs: 0,
        signal_logs: 0,
        log_samples: Vec::new(),
        cost_model,
        paper_fills: 0,
        unfilled_intents: 0,
        filled_notional_quote: 0.0,
        fees_quote: 0.0,
        slippage_cost_quote: 0.0,
        realized_gross_pnl_quote: 0.0,
        unrealized_gross_pnl_quote: 0.0,
        gross_pnl_quote: 0.0,
        net_pnl_quote: 0.0,
        ending_position_qty: 0.0,
        ending_mark_price: None,
        performance: ReplayPerformance {
            initial_equity_quote,
            ending_equity_quote: initial_equity_quote,
            net_return_fraction: 0.0,
            max_drawdown_fraction: 0.0,
            observations: 0,
            mean_return_per_observation: 0.0,
            volatility_per_observation: 0.0,
            sharpe_per_observation: None,
            sortino_per_observation: None,
        },
    };
    let mut book = ReplayBook::default();
    let mut portfolio = PaperPortfolio::default();
    let mut equity_tracker = EquityTracker::new(initial_equity_quote);
    let mut previous_timestamp_ms = None;

    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line_no = index + 1;
        let line = line.map_err(|source| ReplayError::Read {
            line: line_no,
            source,
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let event: ReplayEvent =
            serde_json::from_str(&line).map_err(|error| ReplayError::Invalid {
                line: line_no,
                reason: error.to_string(),
            })?;
        let version = match &event {
            ReplayEvent::Trade { schema_version, .. }
            | ReplayEvent::Depth { schema_version, .. }
            | ReplayEvent::Eval { schema_version, .. } => *schema_version,
        };
        if version != SCHEMA_VERSION {
            return Err(ReplayError::Invalid {
                line: line_no,
                reason: format!("unsupported schema_version {version}; expected {SCHEMA_VERSION}"),
            });
        }

        let timestamp_ms = event_timestamp_ms(&event);
        let time = replay_time(line_no, timestamp_ms)?;
        if let Some(previous) = previous_timestamp_ms {
            // Capture timestamps are millisecond-resolution, so simultaneous
            // events are valid.  A backwards event, however, makes the replay
            // dependent on file order rather than recorded market time.
            if timestamp_ms < previous {
                return Err(ReplayError::Invalid {
                    line: line_no,
                    reason: format!(
                        "timestamp_ms moved backwards: {timestamp_ms} is before previous event {previous}"
                    ),
                });
            }
        }
        previous_timestamp_ms = Some(timestamp_ms);

        match event {
            ReplayEvent::Trade {
                price,
                qty,
                side,
                trade_id,
                ..
            } => {
                validate_positive(line_no, "price", price)?;
                validate_positive(line_no, "qty", qty)?;
                runtime.feed_trade(Trade {
                    price,
                    qty,
                    time,
                    side: side.into(),
                    trade_id,
                });
                book.last_trade = Some(price);
                summary.trades += 1;
            }
            ReplayEvent::Depth { bids, asks, .. } => {
                let depth = Depth {
                    bids: parse_levels(line_no, "bids", bids)?,
                    asks: parse_levels(line_no, "asks", asks)?,
                };
                validate_book(line_no, &depth)?;
                book.best_bid = depth.bids.first().map(|level| level.price);
                book.best_ask = depth.asks.first().map(|level| level.price);
                runtime.feed_depth(depth);
                summary.depth_snapshots += 1;
            }
            ReplayEvent::Eval { .. } => {
                runtime.feed_eval();
                summary.eval_ticks += 1;
            }
        }
        summary.events += 1;
        // Fills may invoke `on_fill` and produce further intents. Bound this
        // loop so a malformed strategy cannot make replay unbounded.
        for _ in 0..1024 {
            let orders: Vec<_> = orders_rx.try_iter().collect();
            if orders.is_empty() {
                break;
            }
            summary.record_orders(&mut runtime, orders.into_iter(), book, &mut portfolio, time);
        }
        if orders_rx.try_recv().is_ok() {
            return Err(ReplayError::CostModel(
                "strategy emitted more than 1024 cascading paper fills for one replay event".into(),
            ));
        }
        for log in runtime.dump_vm_logs() {
            summary.strategy_logs += 1;
            if log.to_ascii_lowercase().contains("signal") {
                summary.signal_logs += 1;
            }
            if summary.log_samples.len() < 32 {
                summary.log_samples.push(log);
            }
        }
        let mark_price = book.mark_price();
        let equity_quote = initial_equity_quote
            + portfolio.realized_gross_pnl_quote
            + portfolio.unrealized_gross_pnl(mark_price)
            - summary.fees_quote;
        equity_tracker.observe(equity_quote);
    }
    summary.finalize_pnl(&portfolio, book.mark_price());
    summary.performance = equity_tracker.finish(initial_equity_quote + summary.net_pnl_quote);
    Ok(summary)
}

fn event_timestamp_ms(event: &ReplayEvent) -> i64 {
    match event {
        ReplayEvent::Trade { timestamp_ms, .. }
        | ReplayEvent::Depth { timestamp_ms, .. }
        | ReplayEvent::Eval { timestamp_ms, .. } => *timestamp_ms,
    }
}

fn replay_time(line: usize, timestamp_ms: i64) -> Result<chrono::DateTime<Utc>, ReplayError> {
    Utc.timestamp_millis_opt(timestamp_ms)
        .single()
        .ok_or_else(|| ReplayError::Invalid {
            line,
            reason: "timestamp_ms is out of range".into(),
        })
}

fn parse_levels(
    line: usize,
    name: &str,
    levels: Vec<ReplayLevel>,
) -> Result<Vec<DepthLevel>, ReplayError> {
    levels
        .into_iter()
        .map(|level| {
            validate_positive(line, &format!("{name}.price"), level.price)?;
            validate_positive(line, &format!("{name}.qty"), level.qty)?;
            Ok(DepthLevel {
                price: level.price,
                qty: level.qty,
            })
        })
        .collect()
}

fn validate_book(line: usize, depth: &Depth) -> Result<(), ReplayError> {
    if depth.bids.is_empty() || depth.asks.is_empty() {
        return Err(ReplayError::Invalid {
            line,
            reason: "depth snapshots require at least one bid and one ask".into(),
        });
    }
    validate_side_order(line, "bids", &depth.bids, true)?;
    validate_side_order(line, "asks", &depth.asks, false)?;

    let best_bid = depth.bids[0].price;
    let best_ask = depth.asks[0].price;
    if best_bid >= best_ask {
        return Err(ReplayError::Invalid {
            line,
            reason: format!(
                "crossed or locked depth: best bid {best_bid} must be below best ask {best_ask}"
            ),
        });
    }
    Ok(())
}

fn validate_side_order(
    line: usize,
    name: &str,
    levels: &[DepthLevel],
    descending: bool,
) -> Result<(), ReplayError> {
    for window in levels.windows(2) {
        let valid = if descending {
            window[0].price > window[1].price
        } else {
            window[0].price < window[1].price
        };
        if !valid {
            let expected = if descending {
                "strictly descending"
            } else {
                "strictly ascending"
            };
            return Err(ReplayError::Invalid {
                line,
                reason: format!("{name} prices must be {expected}"),
            });
        }
    }
    Ok(())
}

fn validate_positive(line: usize, field: &str, value: f64) -> Result<(), ReplayError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(ReplayError::Invalid {
            line,
            reason: format!("{field} must be finite and positive"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    fn fixture(contents: &str) -> String {
        let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("quince-replay-{}-{id}.jsonl", std::process::id()));
        File::create(&path)
            .unwrap()
            .write_all(contents.as_bytes())
            .unwrap();
        path.to_string_lossy().into_owned()
    }

    fn strategy() -> &'static str {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../strategies/test_all.qfl")
    }

    fn strategy_fixture(contents: &str) -> String {
        let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("quince-replay-{}-{id}.qfl", std::process::id()));
        File::create(&path)
            .unwrap()
            .write_all(contents.as_bytes())
            .unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn equity_tracker_reports_drawdown_and_non_annualized_risk_metrics() {
        let mut tracker = EquityTracker::new(100.0);
        tracker.observe(110.0);
        tracker.observe(99.0);
        tracker.observe(104.0);
        let metrics = tracker.finish(104.0);
        assert_eq!(metrics.initial_equity_quote, 100.0);
        assert_eq!(metrics.ending_equity_quote, 104.0);
        assert!((metrics.max_drawdown_fraction - 0.1).abs() < 1e-12);
        assert_eq!(metrics.observations, 3);
        assert!(metrics.sharpe_per_observation.is_some());
        assert!(metrics.sortino_per_observation.is_some());
    }

    #[test]
    fn replay_is_deterministic_for_the_same_capture() {
        let path = fixture(
            r#"{"schema_version":1,"type":"trade","timestamp_ms":1700000000000,"price":100.0,"qty":1.0,"side":"buy","trade_id":1}
{"schema_version":1,"type":"depth","timestamp_ms":1700000000001,"bids":[{"price":99.0,"qty":3.0}],"asks":[{"price":101.0,"qty":2.0}]}
{"schema_version":1,"type":"eval","timestamp_ms":1700000000002}
"#,
        );
        let first = run(strategy(), &path, "BTCUSDT").unwrap();
        let second = run(strategy(), &path, "BTCUSDT").unwrap();
        assert_eq!(first, second);
        assert_eq!(first.events, 3);
        assert_eq!(first.trades, 1);
        assert_eq!(first.depth_snapshots, 1);
        assert_eq!(first.eval_ticks, 1);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn replay_rejects_unknown_schema() {
        let path = fixture(r#"{"schema_version":2,"type":"eval","timestamp_ms":0}"#);
        let error = run(strategy(), &path, "BTCUSDT").unwrap_err();
        assert!(error.to_string().contains("unsupported schema_version 2"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn replay_rejects_non_finite_prices() {
        let path = fixture(
            r#"{"schema_version":1,"type":"trade","timestamp_ms":0,"price":0.0,"qty":1.0,"side":"buy","trade_id":1}"#,
        );
        let error = run(strategy(), &path, "BTCUSDT").unwrap_err();
        assert!(error
            .to_string()
            .contains("price must be finite and positive"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn replay_requires_timestamps_for_depth_and_eval_events() {
        for event in [
            r#"{"schema_version":1,"type":"depth","bids":[{"price":99.0,"qty":1.0}],"asks":[{"price":101.0,"qty":1.0}]}"#,
            r#"{"schema_version":1,"type":"eval"}"#,
        ] {
            let path = fixture(event);
            let error = run(strategy(), &path, "BTCUSDT").unwrap_err();
            assert!(error.to_string().contains("missing field `timestamp_ms`"));
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn replay_rejects_timeline_that_moves_backwards() {
        let path = fixture(
            r#"{"schema_version":1,"type":"trade","timestamp_ms":1700000000001,"price":100.0,"qty":1.0,"side":"buy","trade_id":1}
{"schema_version":1,"type":"eval","timestamp_ms":1700000000000}
"#,
        );
        let error = run(strategy(), &path, "BTCUSDT").unwrap_err();
        assert!(error.to_string().contains("timestamp_ms moved backwards"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn replay_allows_events_with_same_millisecond_timestamp() {
        let path = fixture(
            r#"{"schema_version":1,"type":"trade","timestamp_ms":1700000000000,"price":100.0,"qty":1.0,"side":"buy","trade_id":1}
{"schema_version":1,"type":"eval","timestamp_ms":1700000000000}
"#,
        );
        assert_eq!(run(strategy(), &path, "BTCUSDT").unwrap().events, 2);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn replay_rejects_unsorted_or_crossed_depth() {
        for (capture, expected) in [
            (
                r#"{"schema_version":1,"type":"depth","timestamp_ms":0,"bids":[{"price":99.0,"qty":1.0},{"price":100.0,"qty":1.0}],"asks":[{"price":101.0,"qty":1.0}]}"#,
                "bids prices must be strictly descending",
            ),
            (
                r#"{"schema_version":1,"type":"depth","timestamp_ms":0,"bids":[{"price":99.0,"qty":1.0}],"asks":[{"price":101.0,"qty":1.0},{"price":100.0,"qty":1.0}]}"#,
                "asks prices must be strictly ascending",
            ),
            (
                r#"{"schema_version":1,"type":"depth","timestamp_ms":0,"bids":[{"price":101.0,"qty":1.0}],"asks":[{"price":101.0,"qty":1.0}]}"#,
                "crossed or locked depth",
            ),
        ] {
            let path = fixture(capture);
            let error = run(strategy(), &path, "BTCUSDT").unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn replay_counts_order_intents_without_an_exchange() {
        let strategy = strategy_fixture("function on_eval()\n    quince.order(1, 0.25, 0)\nend\n");
        let path = fixture(
            r#"{"schema_version":1,"type":"trade","timestamp_ms":1700000000000,"price":100.0,"qty":1.0,"side":"buy","trade_id":1}
{"schema_version":1,"type":"eval","timestamp_ms":1700000000001}"#,
        );
        let summary = run(&strategy, &path, "BTCUSDT").unwrap();
        assert_eq!(summary.order_intents, 1);
        assert_eq!(summary.buy_intents, 0);
        assert_eq!(summary.sell_intents, 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(strategy).unwrap();
    }

    #[test]
    fn paper_execution_applies_slippage_fees_and_realized_pnl() {
        let strategy = strategy_fixture(
            r#"
function on_eval()
    quince.order(1, 1.0, 0)
end
"#,
        );
        let path = fixture(
            r#"{"schema_version":1,"type":"trade","timestamp_ms":1700000000000,"price":100.0,"qty":1.0,"side":"buy","trade_id":1}
{"schema_version":1,"type":"eval","timestamp_ms":1700000000001}
"#,
        );
        let summary = run_with_cost_model(
            &strategy,
            &path,
            "BTCUSDT",
            ReplayCostModel {
                fee_bps: 10.0,
                slippage_bps: 5.0,
            },
        )
        .unwrap();
        assert_eq!(summary.paper_fills, 1, "{summary:?}");
        assert_eq!(summary.unfilled_intents, 0);
        assert_eq!(summary.ending_position_qty, -1.0);
        assert!(summary.fees_quote > 0.0);
        assert!(summary.slippage_cost_quote > 0.0);
        assert!(summary.net_pnl_quote < 0.0);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(strategy).unwrap();
    }

    #[test]
    fn non_marketable_limit_is_reported_unfilled() {
        let strategy = strategy_fixture("function on_eval() quince.order(0, 1.0, 100.0, 1) end");
        let path = fixture(
            r#"{"schema_version":1,"type":"depth","timestamp_ms":1700000000000,"bids":[{"price":99.0,"qty":3.0}],"asks":[{"price":101.0,"qty":2.0}]}
{"schema_version":1,"type":"eval","timestamp_ms":1700000000001}
"#,
        );
        let summary =
            run_with_cost_model(&strategy, &path, "BTCUSDT", ReplayCostModel::default()).unwrap();
        assert_eq!(summary.paper_fills, 0);
        assert_eq!(summary.unfilled_intents, 1);
        assert_eq!(summary.net_pnl_quote, 0.0);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(strategy).unwrap();
    }

    #[test]
    fn at_touch_buy_limit_is_never_filled_above_its_limit() {
        let strategy = strategy_fixture("function on_eval() quince.order(0, 1.0, 101.0, 1) end");
        let path = fixture(
            r#"{"schema_version":1,"type":"depth","timestamp_ms":1700000000000,"bids":[{"price":99.0,"qty":3.0}],"asks":[{"price":101.0,"qty":2.0}]}
{"schema_version":1,"type":"eval","timestamp_ms":1700000000001}
"#,
        );
        let summary = run_with_cost_model(
            &strategy,
            &path,
            "BTCUSDT",
            ReplayCostModel {
                fee_bps: 0.0,
                slippage_bps: 100.0,
            },
        )
        .unwrap();
        assert_eq!(summary.paper_fills, 1, "{summary:?}");
        assert_eq!(summary.filled_notional_quote, 101.0);
        assert_eq!(summary.slippage_cost_quote, 0.0);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(strategy).unwrap();
    }

    #[test]
    fn at_touch_sell_limit_is_never_filled_below_its_limit() {
        let strategy = strategy_fixture("function on_eval() quince.order(1, 1.0, 99.0, 1) end");
        let path = fixture(
            r#"{"schema_version":1,"type":"depth","timestamp_ms":1700000000000,"bids":[{"price":99.0,"qty":3.0}],"asks":[{"price":101.0,"qty":2.0}]}
{"schema_version":1,"type":"eval","timestamp_ms":1700000000001}
"#,
        );
        let summary = run_with_cost_model(
            &strategy,
            &path,
            "BTCUSDT",
            ReplayCostModel {
                fee_bps: 0.0,
                slippage_bps: 100.0,
            },
        )
        .unwrap();
        assert_eq!(summary.paper_fills, 1, "{summary:?}");
        assert_eq!(summary.filled_notional_quote, 99.0);
        assert_eq!(summary.slippage_cost_quote, 0.0);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(strategy).unwrap();
    }
}
