use crossbeam_channel;
use mlua::{Function, Lua, Table};
use quince_core::types::*;
use std::collections::HashMap;
use std::sync::Arc;

pub enum LuaEvent {
    Trade(Trade),
    Depth(Depth),
    Fill(OrderFill),
    Eval,
}

pub fn spawn_strategy(
    path: &str,
    event_rx: crossbeam_channel::Receiver<LuaEvent>,
    ctx: Arc<std::sync::Mutex<StrategyCtx>>,
) -> Result<(), String> {
    let code = std::fs::read_to_string(path)
        .map_err(|e| format!("read {}: {}", path, e))?;
    let path = path.to_string();

    std::thread::spawn(move || {
        let lua = Lua::new();

        if let Err(e) = setup_api(&lua, Arc::clone(&ctx)) {
            tracing::error!("lua api setup: {}", e);
            return;
        }

        if let Err(e) = lua.load(&code).exec() {
            tracing::error!("lua exec {}: {}", path, e);
            return;
        }

        let has_on_trade = lua.globals().get::<Function>("on_trade").is_ok();
        let has_on_depth = lua.globals().get::<Function>("on_depth").is_ok();
        let has_on_fill = lua.globals().get::<Function>("on_fill").is_ok();
        let has_on_eval = lua.globals().get::<Function>("on_eval").is_ok();

        while let Ok(event) = event_rx.recv() {
            match event {
                LuaEvent::Trade(trade) if has_on_trade => {
                    if let Ok(t) = trade_to_table(&lua, &trade) {
                        if let Ok(f) = lua.globals().get::<Function>("on_trade") {
                            let _ = f.call::<()>(t);
                        }
                    }
                }
                LuaEvent::Depth(depth) if has_on_depth => {
                    if let Ok(d) = depth_to_table(&lua, &depth) {
                        if let Ok(f) = lua.globals().get::<Function>("on_depth") {
                            let _ = f.call::<()>(d);
                        }
                    }
                }
                LuaEvent::Fill(fill) if has_on_fill => {
                    if let Ok(f) = fill_to_table(&lua, &fill) {
                        if let Ok(func) = lua.globals().get::<Function>("on_fill") {
                            let _ = func.call::<()>(f);
                        }
                    }
                }
                LuaEvent::Eval if has_on_eval => {
                    if let Ok(f) = lua.globals().get::<Function>("on_eval") {
                        let _ = f.call::<()>(());
                    }
                }
                _ => {}
            }
        }
    });

    Ok(())
}

fn setup_api(lua: &Lua, ctx: Arc<std::sync::Mutex<StrategyCtx>>) -> Result<(), mlua::Error> {
    let table = lua.create_table()?;

    let ctx2 = Arc::clone(&ctx);
    let order_fn = lua.create_function(move |_, args: Table| {
        let side: String = args.get("side").unwrap_or_default();
        let qty: f64 = args.get("qty").unwrap_or(0.0);
        let price: Option<f64> = args.get("price").ok();
        let typ: String = args.get("type").unwrap_or("market".into());
        let reduce: bool = args.get("reduce_only").unwrap_or(false);
        let sl: Option<f64> = args.get("stop_loss").ok();
        let tp: Option<f64> = args.get("take_profit").ok();

        let order = Order {
            symbol: "btcusdt".to_string(),
            side: if side == "buy" { Side::Buy } else { Side::Sell },
            qty,
            price,
            order_type: if typ == "limit" { OrderType::Limit } else { OrderType::Market },
            reduce_only: reduce,
            stop_loss: sl,
            take_profit: tp,
        };

        if let Ok(ctx) = ctx2.lock() {
            let _ = ctx.orders_tx.send(order);
        }
        Ok("pending".to_string())
    })?;
    table.raw_set("order", order_fn)?;

    let ctx_b = Arc::clone(&ctx);
    let bal_fn = lua.create_function(move |_, asset: String| {
        let ctx = ctx_b.lock().unwrap();
        Ok(ctx.balance.get(&asset).copied().unwrap_or(0.0))
    })?;
    table.raw_set("balance", bal_fn)?;

    let ctx_p = Arc::clone(&ctx);
    let pos_fn = lua.create_function(move |lua, _symbol: String| {
        let ctx = ctx_p.lock().unwrap();
        match &ctx.position {
            Some(p) => {
                let t = lua.create_table()?;
                t.raw_set("size", p.size)?;
                t.raw_set("entry_price", p.entry_price)?;
                t.raw_set("side", format!("{:?}", p.side))?;
                t.raw_set("unrealized_pnl", p.unrealized_pnl)?;
                Ok(mlua::Value::Table(t))
            }
            None => Ok(mlua::Value::Nil),
        }
    })?;
    table.raw_set("position", pos_fn)?;

    let ctx_pr = Arc::clone(&ctx);
    let price_fn = lua.create_function(move |_, ()| {
        let ctx = ctx_pr.lock().unwrap();
        Ok(ctx.trades.last().map(|t| t.price).unwrap_or(0.0))
    })?;
    table.raw_set("price", price_fn)?;

    let ctx_t = Arc::clone(&ctx);
    let trades_fn = lua.create_function(move |lua, n: usize| {
        let ctx = ctx_t.lock().unwrap();
        let start = ctx.trades.len().saturating_sub(n);
        let t = lua.create_table()?;
        for (i, trade) in ctx.trades[start..].iter().enumerate() {
            let row = lua.create_table()?;
            row.raw_set("price", trade.price)?;
            row.raw_set("qty", trade.qty)?;
            row.raw_set("side", format!("{:?}", trade.side))?;
            row.raw_set("time", trade.time.to_rfc3339())?;
            t.raw_set(i + 1, row)?;
        }
        Ok(t)
    })?;
    table.raw_set("trades", trades_fn)?;

    let ctx_d = Arc::clone(&ctx);
    let depth_fn = lua.create_function(move |lua, ()| {
        let ctx = ctx_d.lock().unwrap();
        match &ctx.depth {
            Some(d) => {
                let t = lua.create_table()?;
                let bids = lua.create_table()?;
                for (i, lvl) in d.bids.iter().enumerate() {
                    let row = lua.create_table()?;
                    row.raw_set("price", lvl.price)?;
                    row.raw_set("qty", lvl.qty)?;
                    bids.raw_set(i + 1, row)?;
                }
                t.raw_set("bids", bids)?;
                let asks = lua.create_table()?;
                for (i, lvl) in d.asks.iter().enumerate() {
                    let row = lua.create_table()?;
                    row.raw_set("price", lvl.price)?;
                    row.raw_set("qty", lvl.qty)?;
                    asks.raw_set(i + 1, row)?;
                }
                t.raw_set("asks", asks)?;
                Ok(mlua::Value::Table(t))
            }
            None => Ok(mlua::Value::Nil),
        }
    })?;
    table.raw_set("depth", depth_fn)?;

    let ctx_i = Arc::clone(&ctx);
    let get_fn = lua.create_function(move |_, name: String| {
        let ctx = ctx_i.lock().unwrap();
        Ok(ctx.indicators.get(name.as_str()).copied().unwrap_or(0.0))
    })?;
    table.raw_set("get", get_fn)?;

    let log_fn = lua.create_function(|_, msg: String| {
        tracing::info!("[lua] {}", msg);
        Ok(())
    })?;
    table.raw_set("log", log_fn)?;

    let time_fn = lua.create_function(|_, ()| {
        Ok(chrono::Utc::now().to_rfc3339())
    })?;
    table.raw_set("time", time_fn)?;

    lua.globals().raw_set("quince", table)?;
    Ok(())
}

fn trade_to_table(lua: &Lua, trade: &Trade) -> Result<Table, mlua::Error> {
    let t = lua.create_table()?;
    t.raw_set("price", trade.price)?;
    t.raw_set("qty", trade.qty)?;
    t.raw_set("side", format!("{:?}", trade.side))?;
    t.raw_set("time", trade.time.to_rfc3339())?;
    t.raw_set("id", trade.trade_id)?;
    Ok(t)
}

fn depth_to_table(lua: &Lua, depth: &Depth) -> Result<Table, mlua::Error> {
    let d = lua.create_table()?;
    let bids = lua.create_table()?;
    for (i, l) in depth.bids.iter().enumerate() {
        let row = lua.create_table()?;
        row.raw_set("price", l.price)?;
        row.raw_set("qty", l.qty)?;
        bids.raw_set(i + 1, row)?;
    }
    d.raw_set("bids", bids)?;
    let asks = lua.create_table()?;
    for (i, l) in depth.asks.iter().enumerate() {
        let row = lua.create_table()?;
        row.raw_set("price", l.price)?;
        row.raw_set("qty", l.qty)?;
        asks.raw_set(i + 1, row)?;
    }
    d.raw_set("asks", asks)?;
    Ok(d)
}

fn fill_to_table(lua: &Lua, fill: &OrderFill) -> Result<Table, mlua::Error> {
    let f = lua.create_table()?;
    f.raw_set("order_id", fill.order_id.clone())?;
    f.raw_set("side", format!("{:?}", fill.side))?;
    f.raw_set("price", fill.price)?;
    f.raw_set("qty", fill.qty)?;
    f.raw_set("fee", fill.fee)?;
    f.raw_set("fee_asset", fill.fee_asset.clone())?;
    f.raw_set("time", fill.time.to_rfc3339())?;
    Ok(f)
}

pub struct StrategyCtx {
    pub trades: Vec<Trade>,
    pub depth: Option<Depth>,
    pub indicators: HashMap<&'static str, f64>,
    pub balance: HashMap<String, f64>,
    pub position: Option<Position>,
    pub orders_tx: crossbeam_channel::Sender<Order>,
}
