# Writing a native indicator

Each native plugin is one Rust file in `indicators/src/custom/`. The build
script discovers those files in deterministic filename order and compiles a
static registry into the binary.

## Contract

Every indicator declares a name, ordered numeric parameters, and a factory.
It receives a public `Trade` and may yield one finite `f64` value. Returning
`None` expresses warm-up; it is not an error. The update method must not block
or allocate on the hot path.

```rust
use quince_core::types::Trade;
use quince_indicators::{CustomIndicator, CustomIndicatorError};

struct MyIndicator;

impl CustomIndicator for MyIndicator {
    fn on_trade(&mut self, trade: &Trade) -> Option<f64> {
        (trade.price.is_finite() && trade.price > 0.0).then_some(trade.price)
    }
}
```

Use `CustomIndicatorRegistration::validate_params` in the factory before
constructing state. The engine independently validates the exact `@using`
arguments at strategy startup, so malformed configurations fail before any live
connection or order intent.

## QFL surface

For a descriptor named `custom_example` with one `period` parameter:

```qfl
@using custom_example:20

on trade(t) {
    feature value = quince.get("custom_example")
}
```

Names are lowercase ASCII identifiers with digits and underscores allowed after
the first character. All current plugins consume trades and expose one scalar
value. The name in `quince.get` must exactly match the descriptor name.

## Required checks

Add focused deterministic tests for warm-up, expected output, and invalid
parameters. Before merging, run:

```bash
cargo +nightly fmt --all -- --check
cargo test --workspace --lib --bins --tests --examples --locked
cargo clippy --workspace --all-targets --no-deps -- -D warnings
```

For a performance-sensitive indicator, add or run a Criterion scenario against
the full QFL pipeline, then compare it to the versioned baseline in CI.
