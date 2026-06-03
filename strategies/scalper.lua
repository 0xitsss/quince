-- HFT scalper: mean reversion on price z-score
-- Target: 0.02-0.03% per scalp
-- USING zscore 20; SET BUFFER 512
-- USING cvd

local in_position = false
local entry_price = 0
local position_side = ""
local pending_entry = ""
local pending_close = false

function on_trade(trade)
    -- Priority 1: exit check (tight P&L on every tick)
    if in_position and entry_price > 0 then
        local pnl_pct, close_side
        if position_side == "long" then
            pnl_pct = (trade.price - entry_price) / entry_price * 100
            close_side = "sell"
        else
            pnl_pct = (entry_price - trade.price) / entry_price * 100
            close_side = "buy"
        end

        if pnl_pct > 0.025 then
            in_position = false; entry_price = 0; position_side = ""
            pending_close = true
            quince.order({side = close_side, qty = 0.001, type = "market", reduce_only = true})
            quince.log("TP +" .. string.format("%.3f", pnl_pct) .. "% @" .. trade.price)
        elseif pnl_pct < -0.03 then
            in_position = false; entry_price = 0; position_side = ""
            pending_close = true
            quince.order({side = close_side, qty = 0.001, type = "market", reduce_only = true})
            quince.log("SL " .. string.format("%.3f", pnl_pct) .. "% @" .. trade.price)
        end
        return
    end

    -- Priority 2: entry signal (tick-level, not waiting for eval)
    if not in_position and pending_entry == "" and not pending_close then
        local z = quince.get("zscore")
        if z == 0 then return end

        if z < -2.0 then
            pending_entry = "buy"
            quince.order({side = "buy", qty = 0.001, type = "market", stop_loss = trade.price * 0.9997, take_profit = trade.price * 1.00025})
            quince.log("BUY z=" .. string.format("%.2f", z) .. " @" .. trade.price)
        elseif z > 2.0 then
            pending_entry = "sell"
            quince.order({side = "sell", qty = 0.001, type = "market", stop_loss = trade.price * 1.0003, take_profit = trade.price * 0.99975})
            quince.log("SELL z=" .. string.format("%.2f", z) .. " @" .. trade.price)
        end
    end
end

function on_fill(fill)
    quince.log("FILL " .. fill.side .. " " .. fill.qty .. " @" .. fill.price)
    if pending_entry == "buy" and fill.side == "Buy" then
        in_position = true; entry_price = fill.price; position_side = "long"; pending_entry = ""
        quince.log("=> LONG @" .. fill.price)
    elseif pending_entry == "sell" and fill.side == "Sell" then
        in_position = true; entry_price = fill.price; position_side = "short"; pending_entry = ""
        quince.log("=> SHORT @" .. fill.price)
    elseif pending_close then
        pending_close = false
    end
end

function on_eval()
end

function on_depth(depth)
end
