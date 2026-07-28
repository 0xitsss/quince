# Native indicator catalogue

All entries below are compiled into the current binary. They consume public
trades and publish one finite `f64` through `quince.get("<name>")`. A dash in
**Parameters** means that the directive takes no arguments. `period` is the
lookback length in trades; `alpha` is an exponential smoothing factor.

```qfl
@using custom_ema:20

on trade(t) {
    feature ema = quince.get("custom_ema")
}
```

Some indicators require warm-up and therefore do not yield a value until enough
trades have arrived. Statistical and microstructure features are descriptive;
they must be replay-tested rather than interpreted as a standalone order signal.

## Trend and momentum

| Indicator | Parameters | Description |
|---|---:|---|
| `custom_sma` | `period` | Simple moving average of trade price; a slow, stable price baseline. |
| `custom_ema` | `period` | Exponentially weighted moving average of price, with more weight on recent trades. |
| `custom_wma` | `period` | Linearly weighted moving average that emphasizes newer prices. |
| `custom_dema` | `period` | Double EMA, reducing lag relative to a single EMA. |
| `custom_tema` | `period` | Triple EMA, a more aggressive lag-reduced moving average. |
| `custom_kama` | `period`, `fast`, `slow` | Kaufman adaptive moving average; adapts smoothing to directional efficiency. |
| `custom_linear_regression` | `period` | Rolling least-squares slope of price, expressing local trend direction. |
| `custom_momentum` | `period` | Difference between current price and the price `period` trades ago. |
| `custom_roc` | `period` | Percentage rate of change over a rolling trade lookback. |
| `custom_rsi` | `period` | Relative Strength Index computed from trade-to-trade gains and losses. |
| `custom_cmo` | `period` | Chande Momentum Oscillator, a signed gain/loss momentum measure. |
| `custom_macd_signal` | `fast`, `slow`, `signal` | MACD signal line derived from fast and slow EMAs of price. |
| `custom_trix` | `period` | Rate of change of a triple-smoothed EMA; suppresses short-term noise. |
| `custom_stochastic_k` | `period` | Current price position within its rolling high-low range. |
| `custom_williams_r` | `period` | Inverted high-low range oscillator indicating price location near extremes. |
| `custom_vortex` | `period` | Directional-movement ratio over a rolling trade window. |
| `custom_efficiency_ratio` | — | Price displacement divided by total absolute path; values near one indicate a clean move. |
| `custom_zscore` | `period` | Price distance from its rolling mean in rolling standard deviations. |

## Volatility and range

| Indicator | Parameters | Description |
|---|---:|---|
| `custom_atr` | `period` | Trade-level average true range, using absolute consecutive price changes. |
| `custom_true_range` | — | Absolute change from the previous trade price. |
| `custom_bollinger_width` | `period` | Width of a rolling price band; a compact proxy for dispersion. |
| `custom_donchian_width` | `period` | Difference between the rolling high and low price. |
| `custom_historical_volatility` | `period` | Rolling volatility of log returns. |
| `custom_ewma_volatility` | `alpha` | Exponentially weighted volatility of log returns. |
| `custom_parkinson_volatility` | `period` | Parkinson-scaled rolling root-mean-square of trade log returns. |
| `custom_return_variance` | — | Online variance of log returns. |
| `custom_return_skewness` | — | Online skewness of log returns; identifies asymmetry in return distribution. |
| `custom_return_kurtosis` | — | Online excess-tailedness measure of log returns. |
| `custom_log_return` | — | Natural logarithm of current price divided by previous price. |

## Volume and money flow

| Indicator | Parameters | Description |
|---|---:|---|
| `signed_volume` | — | Cumulative buy quantity minus sell quantity. |
| `custom_signed_volume_ratio` | — | Signed volume normalized by cumulative total volume. |
| `custom_buy_volume_ratio` | — | Cumulative fraction of traded quantity initiated by buyers. |
| `custom_obv` | — | On-balance volume: volume added or subtracted according to price direction. |
| `custom_cvd` | — | Cumulative volume delta: buy quantity minus sell quantity over time. |
| `custom_vwap` | — | Cumulative volume-weighted average trade price. |
| `custom_vwma` | `period` | Rolling volume-weighted moving average of price. |
| `custom_mfi` | `period` | Money Flow Index based on typical-price changes and traded quantity. |
| `custom_money_flow` | — | Signed typical-price times quantity flow. |
| `custom_force_index` | `alpha` | Smoothed price-change times quantity measure. |
| `custom_chaikin_oscillator` | `fast_alpha`, `slow_alpha` | Difference between fast and slow exponentially smoothed money flow. |
| `custom_volume_roc` | `period` | Percentage rate of change of trade quantity. |
| `custom_volume_zscore` | `period` | Trade quantity relative to its rolling mean and deviation. |
| `custom_large_trade_ratio` | `threshold` | Cumulative share of trades whose quantity meets the threshold. |
| `custom_average_trade_size` | — | Running arithmetic mean of trade quantity. |

## Microstructure and price transforms

| Indicator | Parameters | Description |
|---|---:|---|
| `custom_trade_imbalance` | — | Buy-versus-sell trade-count imbalance. |
| `custom_trade_intensity` | — | Trade arrival intensity estimated from event timestamps. |
| `custom_price_impact` | — | Absolute price move per unit of current trade quantity. |
| `custom_tick_direction` | — | Sign of the most recent trade-to-trade price move. |
| `custom_tick_run_length` | — | Length of the current uninterrupted directional tick run. |
| `custom_median_price` | — | Running midpoint of the observed minimum and maximum trade price. |
| `custom_typical_price` | — | Current trade price, exposed through an explicit custom-indicator contract. |
| `custom_logistic_regression` | `learning_rate`, `l2` | Online logistic model of trade log returns; emits model buy-pressure probability in `[0, 1]`. |

## Selecting a feature

Begin with one feature per hypothesis: trend (`custom_ema` or
`custom_linear_regression`), volatility (`custom_ewma_volatility`), flow
(`custom_cvd` or `custom_buy_volume_ratio`), or microstructure
(`custom_price_impact`). Treat closely related variants as correlated features,
not independent confirmation. Keep a strategy in shadow/replay mode until its
fee- and slippage-adjusted out-of-sample behavior is understood.
