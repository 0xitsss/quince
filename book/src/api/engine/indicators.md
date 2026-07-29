# Module: `indicators`

> Source: `indicators.rs`

Indicator parsing and management for the trading engine.
Parses `@using` directives from QFL strategy headers into [`IndicatorEntry`] lists
and provides [`IndicatorBank`] for runtime indicator lifecycle.

## Structs

### `pub struct IndicatorEntry`

```rust
pub struct IndicatorEntry {
    pub name: String,
    pub params: Vec < f64 >,
    pub buffer: usize,
};
```


### `pub struct IndicatorBank`

```rust
pub struct IndicatorBank {
    indicators: Vec < ActiveIndicator >,
    results: Vec < (u16 , f64) >,
    slot_sma: u16,
    slot_ema: u16,
    slot_wma: u16,
    slot_vwma: u16,
    slot_lsma: u16,
    slot_rsi: u16,
    slot_macd: u16,
    slot_macd_signal: u16,
    slot_macd_histogram: u16,
    slot_cci: u16,
    slot_roc: u16,
    slot_stoch: u16,
    slot_bb_middle: u16,
    slot_bb_upper: u16,
    slot_bb_lower: u16,
    slot_bb_bandwidth: u16,
    slot_kc_middle: u16,
    slot_kc_upper: u16,
    slot_kc_lower: u16,
    slot_atr: u16,
    slot_mfi: u16,
    slot_adx: u16,
    slot_zscore: u16,
    slot_cvd: u16,
    slot_pmdi: u16,
    slot_nmdi: u16,
    slot_price: u16,
    slot_volume_delta: u16,
    slot_avg_trade_size: u16,
    slot_trade_count: u16,
    slot_bid_depth: u16,
    slot_ask_depth: u16,
    slot_depth_imbalance: u16,
    cum_buy: f64,
    cum_sell: f64,
    trades: u64,
};
```



## Functions

### `pub fn parse_using`

```rust
pub fn parse_using(...) { ... }
```


### `pub fn parse_using_strict`

```rust
pub fn parse_using_strict(...) { ... }
```

Strict production parser for `@using` directives.
Unlike [`parse_using`], it rejects malformed numeric parameters instead of
silently dropping them. The engine uses this at startup.

