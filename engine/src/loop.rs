use crate::indicators::{parse_using, IndicatorBank};
use crate::orders::{OrderManager, PendingStatus};
use quince_core::types::*;
use quince_exchange::r#trait::{Exchange, ExchangeError, StreamMsg};
use quince_logger::TradeLog;
use quince_risk::RiskControls;
use quince_strategy::runtime::{spawn_strategy, LuaEvent, StrategyCtx};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

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

    event_tx: mpsc::UnboundedSender<LuaEvent>,
    orders_rx: mpsc::UnboundedReceiver<Order>,
    ctx: Arc<Mutex<StrategyCtx>>,

    risk: RiskControls,
    logger: TradeLog,

    order_manager: OrderManager,
    indicators: IndicatorBank,

    last_price: f64,
    last_depth: Option<Depth>,
    daily_pnl: f64,
    peak_equity: f64,
    order_timestamps: Vec<Instant>,
}

impl<E: Exchange> Engine<E> {
    pub fn new(
        exchange: E,
        symbols: &[String],
        strategy_path: &str,
        risk: RiskControls,
        log_path: &str,
    ) -> Result<Self, EngineError> {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (orders_tx, orders_rx) = mpsc::unbounded_channel();

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

        let src = std::fs::read_to_string(strategy_path)
            .map_err(|e| EngineError::Strategy(format!("read {}: {}", strategy_path, e)))?;
        let ind_cfg = parse_using(&src);
        let indicators = IndicatorBank::new(&ind_cfg);

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
            last_depth: None,
            daily_pnl: 0.0,
            peak_equity: 0.0,
            order_timestamps: Vec::new(),
        })
    }

    pub async fn run(&mut self) -> Result<(), EngineError> {
        let stream = self.exchange.subscribe(&self.symbols).await?;
        let mut rx = stream.rx;
        let mut eval_tick = tokio::time::interval(EVAL_INTERVAL);
        let mut account_tick = tokio::time::interval(ACCOUNT_SYNC_INTERVAL);

        loop {
            tokio::select! {
                Some(msg) = rx.recv() => {
                    self.on_stream_msg(msg).await;
                }
                Some(order) = self.orders_rx.recv() => {
                    self.on_strategy_order(order).await;
                }
                _ = eval_tick.tick() => {
                    self.on_eval().await;
                }
                _ = account_tick.tick() => {
                    self.sync_account().await;
                }
                else => break,
            }

            self.check_timeouts().await;
        }

        Ok(())
    }

    async fn on_stream_msg(&mut self, msg: StreamMsg) {
        match msg {
            StreamMsg::Trade(trade) => {
                self.last_price = trade.price;
                let ind_vals = self.indicators.on_trade(&trade);
                {
                    let mut ctx = self.ctx.lock().unwrap();
                    ctx.trades.push(trade);
                    ctx.indicators.extend(ind_vals);
                    let overflow = ctx.trades.len().saturating_sub(MAX_TRADES);
                    if overflow > 0 {
                        ctx.trades.drain(0..overflow);
                    }
                }
                let _ = self.event_tx.send(LuaEvent::Trade(trade));
            }
            StreamMsg::Depth(depth) => {
                self.last_depth = Some(depth.clone());
                let ind_vals = self.indicators.on_depth(&depth);
                {
                    let mut ctx = self.ctx.lock().unwrap();
                    ctx.indicators.extend(ind_vals);
                    ctx.depth = Some(depth.clone());
                }
                let _ = self.event_tx.send(LuaEvent::Depth(depth));
            }
            StreamMsg::MarkPrice { price, .. } => {
                self.last_price = price;
            }
            StreamMsg::OrderUpdate(fill) => {
                let symbol = self.symbols.first().cloned().unwrap_or_default();
                let matched = self.order_manager.pending_order_ids().iter().any(|cid| {
                    if let Some(po) = self.order_manager.get(cid) {
                        po.order.symbol == symbol
                            && po.order.side == fill.side
                    } else {
                        false
                    }
                });

                if matched {
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
