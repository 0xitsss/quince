# Quince

Quince is a low-latency Rust execution engine and the Quince-Flavored Language
(QFL) runtime for event-driven trading strategies. Its hot path is synchronous,
bounded, and allocation-free after strategy and indicator construction.

The system deliberately separates three concerns:

1. **Market data and execution adapters** normalize exchange events and keep
   live execution fail-closed when account, market-data, or reconciliation
   guarantees are absent.
2. **The engine and QFL VM** compile a strategy once, run it per event, and
   enforce risk and instruction budgets before an order can leave the process.
3. **Indicators** turn public trades into finite scalar features that QFL reads
   with `quince.get("name")`.

## Quick indicator example

The indicator is declared before the strategy handlers and read inside the
handler. A custom indicator has the same QFL surface as a built-in one:

```qfl
@using custom_logistic_regression:0.05:0.01

on trade(t) {
    feature buy_probability = quince.get("custom_logistic_regression")
    if buy_probability > 0.60 {
        quince.log("buy pressure")
    }
}
```

`@using` is validated during startup. Unknown names, a wrong parameter count,
non-numeric arguments, or out-of-range values reject the strategy before it can
execute. An indicator may return no value during warm-up; QFL sees the normal
engine default until it has a finite scalar.

## Native extension model

Custom indicators are Rust source files compiled and linked into the Quince
binary. Dynamic plugins are intentionally not loaded: this makes the deployed
artifact reproducible and ensures every indicator participates in review,
tests, linting, and benchmark gates. See [Writing a native
indicator](writing-indicators.md) for the contract, and the [native catalogue](native-indicators.md)
for all currently linked indicators.

## Validation boundary

An indicator is a feature, not a trading claim. Validate it in the replay
environment with fees, slippage, and out-of-sample data before allowing a
strategy that uses it to progress from shadow mode to execution.
