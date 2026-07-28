# Module: `custom`

> Source: `custom.rs`

Native, compile-time custom-indicator extension API.

Put one Rust source file in `src/custom/`. The build script discovers it at
compile time and adds its [`CustomIndicatorRegistration`] to the registry.
Dynamic loading is deliberately unsupported: every plugin is reviewed,
compiled, and linked into the Quince binary.

## Structs

### `pub struct IndicatorParameter`

```rust
pub struct IndicatorParameter {
    pub name: & 'static str,
    pub min: f64,
    pub max: f64,
};
```

A named numeric parameter accepted by a custom indicator.

### `pub struct IndicatorDescriptor`

```rust
pub struct IndicatorDescriptor {
    pub name: & 'static str,
    pub input: IndicatorInput,
    pub output: IndicatorOutput,
    pub parameters: & 'static [IndicatorParameter],
};
```

Immutable metadata declared by every custom indicator.

### `pub struct CustomIndicatorRegistration`

```rust
pub struct CustomIndicatorRegistration {
    pub descriptor: & 'static IndicatorDescriptor,
    pub create: CustomIndicatorFactory,
};
```

Compile-time registration emitted by a custom-indicator source file.


## Enums

### `pub enum IndicatorInput`

```rust
pub enum IndicatorInput {
    Trade,
}
```

Market-event format accepted by an indicator.

### `pub enum IndicatorOutput`

```rust
pub enum IndicatorOutput {
    ScalarF64,
}
```

Output format exposed to QFL through `quince.get("<name>")`.

### `pub enum CustomIndicatorError`

```rust
pub enum CustomIndicatorError {
    UnknownIndicator(String,),
    InvalidParameterCount(indicator: & 'static str,
    expected: usize,
    actual: usize,),
    InvalidParameter(indicator: & 'static str,
    parameter: & 'static str,
    value: f64,
    min: f64,
    max: f64,),
    Construction(indicator: & 'static str,
    reason: & 'static str,),
}
```

Construction or validation failure for a custom indicator.


## Traits

### `pub trait CustomIndicator`

```rust
pub trait CustomIndicator: Send {
    fn on_trade (& mut self , trade : & Trade) -> Option < f64 >;
}
```

Native indicator implementation. `on_trade` must not allocate or block.


## Functions

### `pub fn custom_indicator`

```rust
pub fn custom_indicator(...) { ... }
```

Finds a compile-time registered custom indicator by its QFL name.

### `pub fn custom_indicators`

```rust
pub fn custom_indicators(...) { ... }
```

All custom indicators linked into this Quince build, in deterministic
filename order. This is intended for startup validation and tooling only.


## Type Aliases

### `pub type CustomIndicatorFactory`

```rust
pub type CustomIndicatorFactory = fn (& [f64]) -> Result < Box < dyn CustomIndicator > , CustomIndicatorError >;
```

Factory signature used by the generated custom-indicator registry.

