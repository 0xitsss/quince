# Quince Book

- [Introduction](README.md)
- [Native indicator catalogue](native-indicators.md)
- [Writing a native indicator](writing-indicators.md)

# API Reference

--DOCGEN:API--

### qfl

- [ast](api/qfl/ast.md)
- [checker](api/qfl/checker.md)
- [compiler](api/qfl/compiler.md)
- [config](api/qfl/config.md)
- [ir](api/qfl/ir.md)
- [lexer](api/qfl/lexer.md)
- [lib](api/qfl/index.md)
- [log_buffer](api/qfl/log_buffer.md)
- [opcodes](api/qfl/opcodes.md)
- [optimize](api/qfl/optimize.md)
- [parser](api/qfl/parser.md)
- [profiler](api/qfl/profiler.md)
- [risk](api/qfl/risk.md)
- [runtime](api/qfl/runtime.md)
- [tracer](api/qfl/tracer.md)
- [types](api/qfl/types.md)
- [vm](api/qfl/vm.md)

### core

- [lib](api/core/index.md)
- [ring](api/core/ring.md)
- [types](api/core/types.md)

### engine

- [indicators](api/engine/indicators.md)
- [journal](api/engine/journal.md)
- [lib](api/engine/index.md)
- [loop](api/engine/loop.md)
- [orders](api/engine/orders.md)
- [strategy_lifecycle](api/engine/strategy_lifecycle.md)
- [telemetry](api/engine/telemetry.md)
- [bin/strategy_bench](api/engine/bin/strategy_bench.md)

### exchange

- [lib](api/exchange/index.md)
- [trait](api/exchange/trait.md)
- [binance/filters](api/exchange/binance/filters.md)
- [binance/mod](api/exchange/binance/mod.md)
- [binance/public](api/exchange/binance/public.md)
- [binance/types](api/exchange/binance/types.md)
- [binance/user_data](api/exchange/binance/user_data.md)
- [binance/ws](api/exchange/binance/ws.md)
- [hyperliquid/execution](api/exchange/hyperliquid/execution.md)
- [hyperliquid/mod](api/exchange/hyperliquid/mod.md)
- [hyperliquid/preflight](api/exchange/hyperliquid/preflight.md)
- [hyperliquid/public](api/exchange/hyperliquid/public.md)
- [hyperliquid/signing](api/exchange/hyperliquid/signing.md)
- [hyperliquid/user_data](api/exchange/hyperliquid/user_data.md)

### indicators

- [custom](api/indicators/custom.md)
- [flow](api/indicators/flow.md)
- [lib](api/indicators/index.md)
- [ma](api/indicators/ma.md)
- [oscillator](api/indicators/oscillator.md)
- [simd](api/indicators/simd.md)
- [structure](api/indicators/structure.md)
- [volatility](api/indicators/volatility.md)
- [custom/custom_atr](api/indicators/custom/custom_atr.md)
- [custom/custom_average_trade_size](api/indicators/custom/custom_average_trade_size.md)
- [custom/custom_bollinger_width](api/indicators/custom/custom_bollinger_width.md)
- [custom/custom_buy_volume_ratio](api/indicators/custom/custom_buy_volume_ratio.md)
- [custom/custom_chaikin_oscillator](api/indicators/custom/custom_chaikin_oscillator.md)
- [custom/custom_cmo](api/indicators/custom/custom_cmo.md)
- [custom/custom_cvd](api/indicators/custom/custom_cvd.md)
- [custom/custom_dema](api/indicators/custom/custom_dema.md)
- [custom/custom_donchian_width](api/indicators/custom/custom_donchian_width.md)
- [custom/custom_efficiency_ratio](api/indicators/custom/custom_efficiency_ratio.md)
- [custom/custom_ema](api/indicators/custom/custom_ema.md)
- [custom/custom_ewma_volatility](api/indicators/custom/custom_ewma_volatility.md)
- [custom/custom_force_index](api/indicators/custom/custom_force_index.md)
- [custom/custom_historical_volatility](api/indicators/custom/custom_historical_volatility.md)
- [custom/custom_kama](api/indicators/custom/custom_kama.md)
- [custom/custom_large_trade_ratio](api/indicators/custom/custom_large_trade_ratio.md)
- [custom/custom_linear_regression](api/indicators/custom/custom_linear_regression.md)
- [custom/custom_log_return](api/indicators/custom/custom_log_return.md)
- [custom/custom_logistic_regression](api/indicators/custom/custom_logistic_regression.md)
- [custom/custom_macd_signal](api/indicators/custom/custom_macd_signal.md)
- [custom/custom_median_price](api/indicators/custom/custom_median_price.md)
- [custom/custom_mfi](api/indicators/custom/custom_mfi.md)
- [custom/custom_momentum](api/indicators/custom/custom_momentum.md)
- [custom/custom_money_flow](api/indicators/custom/custom_money_flow.md)
- [custom/custom_obv](api/indicators/custom/custom_obv.md)
- [custom/custom_parkinson_volatility](api/indicators/custom/custom_parkinson_volatility.md)
- [custom/custom_price_impact](api/indicators/custom/custom_price_impact.md)
- [custom/custom_return_kurtosis](api/indicators/custom/custom_return_kurtosis.md)
- [custom/custom_return_skewness](api/indicators/custom/custom_return_skewness.md)
- [custom/custom_return_variance](api/indicators/custom/custom_return_variance.md)
- [custom/custom_roc](api/indicators/custom/custom_roc.md)
- [custom/custom_rsi](api/indicators/custom/custom_rsi.md)
- [custom/custom_signed_volume_ratio](api/indicators/custom/custom_signed_volume_ratio.md)
- [custom/custom_sma](api/indicators/custom/custom_sma.md)
- [custom/custom_stochastic_k](api/indicators/custom/custom_stochastic_k.md)
- [custom/custom_tema](api/indicators/custom/custom_tema.md)
- [custom/custom_tick_direction](api/indicators/custom/custom_tick_direction.md)
- [custom/custom_tick_run_length](api/indicators/custom/custom_tick_run_length.md)
- [custom/custom_trade_imbalance](api/indicators/custom/custom_trade_imbalance.md)
- [custom/custom_trade_intensity](api/indicators/custom/custom_trade_intensity.md)
- [custom/custom_trix](api/indicators/custom/custom_trix.md)
- [custom/custom_true_range](api/indicators/custom/custom_true_range.md)
- [custom/custom_typical_price](api/indicators/custom/custom_typical_price.md)
- [custom/custom_volume_roc](api/indicators/custom/custom_volume_roc.md)
- [custom/custom_volume_zscore](api/indicators/custom/custom_volume_zscore.md)
- [custom/custom_vortex](api/indicators/custom/custom_vortex.md)
- [custom/custom_vwap](api/indicators/custom/custom_vwap.md)
- [custom/custom_vwma](api/indicators/custom/custom_vwma.md)
- [custom/custom_williams_r](api/indicators/custom/custom_williams_r.md)
- [custom/custom_wma](api/indicators/custom/custom_wma.md)
- [custom/custom_zscore](api/indicators/custom/custom_zscore.md)
- [custom/signed_volume](api/indicators/custom/signed_volume.md)

### logger

- [lib](api/logger/index.md)

### risk

- [controls](api/risk/controls.md)
- [lib](api/risk/index.md)

### quince

- [capture_merge](api/quince/capture_merge.md)
- [dashboard](api/quince/dashboard.md)
- [lib](api/quince/index.md)
- [main](api/quince/main.md)
- [mock](api/quince/mock.md)
- [okx_import](api/quince/okx_import.md)
- [replay](api/quince/replay.md)
- [replay_suite](api/quince/replay_suite.md)
- [wallet](api/quince/wallet.md)
- [bin/dump_qfl](api/quince/bin/dump_qfl.md)
