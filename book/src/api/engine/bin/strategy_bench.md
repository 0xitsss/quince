# Module: `bin/strategy_bench`

> Source: `bin/strategy_bench.rs`

Reproducible QFL strategy latency matrix.

Measures the single-threaded hot path: indicator update, indicator-slot
writes and `on_trade` VM dispatch. Networking, disk I/O, logging sinks and
exchange acknowledgements are deliberately outside the measurement.
