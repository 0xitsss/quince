# Module: `ir`

> Source: `ir.rs`

QFL IR (Intermediate Representation) — serializable bytecode format.

Defines [`QfrProgram`] (V1/V2), the [`EntryPoint`] table, [`ConstEntry`] pool,
and [`quince_hash64`] checksum. Supports binary serialization/deserialization
with mmap-compatible V2 format.

Entry points: [`save_qfr()`](runtime::QflRuntime::save_qfr), [`load_qfr()`].

## Structs

### `pub struct EntryPoint`

```rust
pub struct EntryPoint {
    pub name: String,
    pub code_offset: u32,
};
```

Legacy entry point (compiler side)

### `pub struct QfrProgram`

```rust
pub struct QfrProgram {
    pub entries: Vec < EntryPoint >,
    pub const_pool: Vec < ConstEntry >,
    pub code: Vec < Instruction >,
    pub const_map: HashMap < String , u32 >,
    pub ema_alphas: Vec < f64 >,
    pub f64_consts: Vec < f64 >,
    pub i64_consts: Vec < i64 >,
    pub string_consts: Vec < String >,
};
```

Legacy program representation used by the compiler

### `pub struct QfrBinarized`

```rust
pub struct QfrBinarized {
    pub magic: [u8 ; 4],
    pub version: u16,
    pub entry_count: u16,
    pub num_constants: u32,
    pub num_instructions: u32,
    pub persist_mask: [u64 ; 4],
    _reserved: [u8 ; 16],
};
```

Binary header — byte-exact layout for memory mapping.
Total header size: 64 bytes (cache-line aligned).

### `pub struct QfrEntry`

```rust
pub struct QfrEntry {
    pub name_offset: u32,
    pub name_len: u32,
    pub code_offset: u32,
    _pad: u32,
};
```

Entry point descriptor in the binary format.

### `pub struct Loader`

```rust
pub struct Loader {
    _mmap: memmap2 :: Mmap,
    pub header: NonNull < QfrBinarized >,
    pub constants_ptr: * const f64,
    pub instructions_ptr: * const u64,
    pub entry_count: u16,
    pub const_count: u32,
    pub instr_count: u32,
};
```

Zero-copy loader — memory-maps a .qfr file and exposes raw pointers.


## Enums

### `pub enum ConstEntry`

```rust
pub enum ConstEntry {
    I64(i64,),
    F64(f64,),
    String(String,),
}
```

Legacy const pool entry (compiler side)


## Functions

### `pub fn quince_hash64`

```rust
pub fn quince_hash64(...) { ... }
```


### `pub fn serialize_binarized`

```rust
pub fn serialize_binarized(...) { ... }
```

Serialize a QfrProgram into the zero-copy mmap-compatible binary format.

### `pub fn deserialize_binarized`

```rust
pub fn deserialize_binarized(...) { ... }
```

Deserialize from binarized format back to QfrProgram (for backward compat).

### `pub fn serialize_v1`

```rust
pub fn serialize_v1(...) { ... }
```


### `pub fn deserialize_v1`

```rust
pub fn deserialize_v1(...) { ... }
```


### `pub fn serialize`

```rust
pub fn serialize(...) { ... }
```


### `pub fn deserialize`

```rust
pub fn deserialize(...) { ... }
```



## Constants

### `pub const QFR_MAGIC_V1`

```rust
pub const QFR_MAGIC_V1: & [u8 ; 4] = ...;
```


### `pub const QFR_MAGIC_V2`

```rust
pub const QFR_MAGIC_V2: & [u8 ; 4] = ...;
```


### `pub const QFRC_MAGIC`

```rust
pub const QFRC_MAGIC: [u8 ; 4] = ...;
```


### `pub const QFRC_FOOTER_SIZE`

```rust
pub const QFRC_FOOTER_SIZE: usize = ...;
```


### `pub const QFR_VERSION_V1`

```rust
pub const QFR_VERSION_V1: u32 = ...;
```


### `pub const QFR_VERSION_V2`

```rust
pub const QFR_VERSION_V2: u16 = ...;
```


