-- USING sma 20
-- USING ema 20
-- USING wma 20
-- USING vwma 20
-- USING lsma 20
-- USING rsi 14
-- USING macd 12 26 9
-- USING cci 20 0.015
-- USING roc 14
-- USING stoch 14
-- USING bb 20 2.0
-- USING kc 20 1.5
-- USING atr 14
-- USING mfi 14
-- USING adx 14
-- USING cvd
-- USING pmdi
-- USING nmdi
-- USING zscore 20

local counter = 0

function on_trade(trade)
    counter = counter + 1
    if counter % 100 ~= 0 then return end

    quince.log("=== trade #" .. counter .. " ===")

    local indicators = {
        "price", "trade_count",
        "sma", "ema", "wma", "vwma", "lsma",
        "rsi", "macd", "macd.signal", "macd.histogram",
        "cci", "roc", "stoch",
        "bb.middle", "bb.upper", "bb.lower", "bb.bandwidth",
        "kc.middle", "kc.upper", "kc.lower",
        "atr", "mfi", "adx",
        "cvd", "pmdi", "nmdi",
        "volume_delta", "avg_trade_size", "zscore",
    }
    for _, name in ipairs(indicators) do
        local val = quince.get(name)
        if val ~= 0 then
            quince.log("  " .. name .. " = " .. val)
        end
    end
    quince.log("")
end

function on_depth(depth)
    quince.log("depth: bid=" .. quince.get("bid_depth") .. " ask=" .. quince.get("ask_depth") .. " imb=" .. quince.get("depth_imbalance"))
end

function on_eval()
end

function on_fill(fill)
end
