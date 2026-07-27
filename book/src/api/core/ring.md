# Module: `ring`

> Source: `ring.rs`

Lock-free ring buffer data structures for zero-allocation fixed-capacity storage.
Provides [`RingBuffer`] (const-generic inline buffer) and [`RingVec`] (heap-allocated)
with O(1) push/pop and optional eviction of oldest elements at capacity.

## Structs

### `pub struct RingBuffer<T, const N : usize>`

```rust
pub struct RingBuffer<T, const N : usize> {
    buf: [MaybeUninit < T > ; N],
    head: usize,
    len: usize,
};
```


### `pub struct RingIter<'a, T, const N : usize>`

```rust
pub struct RingIter<'a, T, const N : usize> {
    buf: & 'a RingBuffer < T , N >,
    pos: usize,
};
```


### `pub struct RingVec`

```rust
pub struct RingVec {
    data: Vec < f64 >,
    head: usize,
    len: usize,
    cap: usize,
};
```


### `pub struct RingVecIter<'a>`

```rust
pub struct RingVecIter<'a> {
    buf: & 'a RingVec,
    pos: usize,
};
```


