local fast_period = 12
local slow_period = 26

local fast_sum = {}
local slow_sum = {}
local prices = {}

local function ema(period, prices_list)
    local k = 2 / (period + 1)
    local sum = 0
    for i = 1, period do
        local idx = #prices_list - period + i
        if idx < 1 then return nil end
        sum = sum + prices_list[idx]
    end
    local ema_val = sum / period
    for i = #prices_list - period + 2, #prices_list do
        ema_val = prices_list[i] * k + ema_val * (1 - k)
    end
    return ema_val
end

function on_trade(trade)
    table.insert(prices, trade.price)
    if #prices > slow_period then
        table.remove(prices, 1)
    end

    if #prices < slow_period then return end

    local fast = ema(fast_period, prices)
    local slow = ema(slow_period, prices)
    if fast == nil or slow == nil then return end

    local pos = quince.position("btcusdt")
    local balance = quince.balance("USDT")

    if fast > slow then
        if pos == nil or pos.size == 0 then
            quince.order({side = "buy", qty = 0.001, type = "market"})
            quince.log("ema cross LONG")
        end
    elseif fast < slow then
        if pos ~= nil and pos.size > 0 then
            quince.order({side = "sell", qty = pos.size, type = "market", reduce_only = true})
            quince.log("ema cross SHORT")
        end
    end
end

function on_depth(depth)
end

function on_fill(fill)
    quince.log("filled " .. fill.qty .. " at " .. fill.price)
end

function on_eval()
    local pos = quince.position("btcusdt")
    if pos ~= nil and pos.size > 0 then
        local entry = pos.entry_price
        local current = quince.price()
        local pnl_pct = (current - entry) / entry * 100
        if pnl_pct < -2.0 then
            quince.order({side = "sell", qty = pos.size, type = "market", reduce_only = true})
            quince.log("stop loss triggered")
        end
    end
end
