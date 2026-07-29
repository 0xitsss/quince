# Module: `binance/filters`

> Source: `binance/filters.rs`

Local validation for Binance `exchangeInfo` symbol filters.

This module deliberately consumes the exchange response instead of carrying
a hand-maintained precision table.  It currently understands the common
`PRICE_FILTER`, `LOT_SIZE`, and `MIN_NOTIONAL`/`NOTIONAL` fields. Binance
represents numeric fields as decimal strings; JSON numbers are accepted as
a convenience for fixtures, but production callers should preserve the
response unchanged.

A zero min/max bound is treated as disabled, matching Binance's documented
filter convention.  Price and quantity normalization floors toward zero to
the permitted increment: normalization never increases an order's price or
exposure. Callers must submit the returned values, not the original input.

## Structs

### `pub struct NormalizedLimitOrder`

```rust
pub struct NormalizedLimitOrder {
    pub symbol: String,
    pub price: f64,
    pub qty: f64,
};
```


### `pub struct SymbolFilters`

```rust
pub struct SymbolFilters {
    symbol: String,
    tick_size: f64,
    tick_precision: usize,
    min_price: Option < f64 >,
    max_price: Option < f64 >,
    step_size: f64,
    qty_precision: usize,
    min_qty: Option < f64 >,
    max_qty: Option < f64 >,
    min_notional: Option < f64 >,
};
```


### `pub struct BinanceFilters`

```rust
pub struct BinanceFilters {
    symbols: HashMap < String , SymbolFilters >,
};
```

Indexes symbol filters parsed from one Binance `exchangeInfo` response.

