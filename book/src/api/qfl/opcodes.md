# Module: `opcodes`

> Source: `opcodes.rs`

QFL opcode definitions and instruction encoding.

Defines the [`Opcode`] enum (70 opcodes), the [`Instruction`] wrapper (u64),
and encoding/decoding helpers (`Ri40`, `RRI`, `RRR`).

Instruction layout: `[opcode:8][rd:8][rs1:8][rs2:8][imm:32]`

## Structs

### `pub struct Instruction`

```rust
pub struct Instruction(u64,);;
```

Raw 64-bit instruction (opcode in bits 0-7 for zero-shift dispatch)


## Enums

### `pub enum InstrEncoding`

```rust
pub enum InstrEncoding {
    RRR,
    RR,
    RRI,
    RI,
    RI40,
    Single,
}
```


### `pub enum Opcode`

```rust
pub enum Opcode {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,
    AddI,
    SubI,
    MulI,
    DivI,
    FAdd,
    FSub,
    FMul,
    FDiv,
    FNeg,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    FEq,
    FNe,
    FLt,
    FGt,
    FLe,
    FGe,
    EqI,
    LtI,
    GtI,
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    Shl,
    Shr,
    Jmp,
    Jz,
    Jnz,
    Call,
    Ret,
    Mov,
    Ldi,
    Ldi64,
    LdcF64,
    I2F,
    F2I,
    GetInd,
    GetPrice,
    GetPos,
    GetBal,
    GetDepthBid,
    GetDepthAsk,
    SendOrder,
    PersistGet,
    PersistSet,
    Log,
    Halt,
    WindowPush,
    WindowMean,
    WindowStddev,
    WindowMin,
    WindowMax,
    WindowSum,
    Ema,
    Log2,
    LdI64,
    LdcStr,
    Pow,
    FPow,
    Sentinel,
}
```



## Constants

### `pub const OPCODE_BITS`

```rust
pub const OPCODE_BITS: u32 = ...;
```


### `pub const REGISTER_BITS`

```rust
pub const REGISTER_BITS: u32 = ...;
```


### `pub const IMM_BITS`

```rust
pub const IMM_BITS: u32 = ...;
```


### `pub const SENTINEL_OPCODE`

```rust
pub const SENTINEL_OPCODE: u8 = ...;
```


