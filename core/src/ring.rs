use core::mem::MaybeUninit;
use core::ptr;

#[repr(C)]
pub struct RingBuffer<T, const N: usize> {
    buf: [MaybeUninit<T>; N],
    head: usize,
    len: usize,
}

impl<T, const N: usize> RingBuffer<T, N> {
    pub const fn new() -> Self {
        Self {
            buf: unsafe { MaybeUninit::uninit().assume_init() },
            head: 0,
            len: 0,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub const fn capacity(&self) -> usize {
        N
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.len == N
    }

    #[inline]
    fn idx(&self, i: usize) -> usize {
        (self.head + i) % N
    }

    pub fn push(&mut self, val: T) {
        if N == 0 {
            return;
        }
        if self.len < N {
            self.buf[self.idx(self.len)] = MaybeUninit::new(val);
            self.len += 1;
        } else {
            let slot = self.head;
            unsafe {
                ptr::drop_in_place(self.buf[slot].as_mut_ptr());
            }
            self.buf[slot] = MaybeUninit::new(val);
            self.head = (self.head + 1) % N;
        }
    }

    pub fn get(&self, i: usize) -> Option<&T> {
        if i < self.len {
            Some(unsafe { self.buf[self.idx(i)].assume_init_ref() })
        } else {
            None
        }
    }

    pub fn last(&self) -> Option<&T> {
        if self.len == 0 {
            None
        } else {
            Some(unsafe { self.buf[self.idx(self.len - 1)].assume_init_ref() })
        }
    }

    pub fn clear(&mut self) {
        for i in 0..self.len {
            unsafe {
                ptr::drop_in_place(self.buf[self.idx(i)].as_mut_ptr());
            }
        }
        self.head = 0;
        self.len = 0;
    }

    pub fn iter(&self) -> RingIter<'_, T, N> {
        RingIter { buf: self, pos: 0 }
    }
}

impl<T, const N: usize> Default for RingBuffer<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> Drop for RingBuffer<T, N> {
    fn drop(&mut self) {
        for i in 0..self.len {
            unsafe {
                ptr::drop_in_place(self.buf[self.idx(i)].as_mut_ptr());
            }
        }
    }
}

pub struct RingIter<'a, T, const N: usize> {
    buf: &'a RingBuffer<T, N>,
    pos: usize,
}

impl<'a, T, const N: usize> Iterator for RingIter<'a, T, N> {
    type Item = &'a T;
    fn next(&mut self) -> Option<Self::Item> {
        let v = self.buf.get(self.pos);
        self.pos += 1;
        v
    }
}

// ── RingVec: heap-allocated ring buffer, фиксированная capacity ──

#[derive(Debug, Clone)]
pub struct RingVec {
    data: Vec<f64>,
    head: usize,
    len: usize,
    cap: usize,
}

impl RingVec {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            data: Vec::with_capacity(cap),
            head: 0,
            len: 0,
            cap,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.cap
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.len == self.cap
    }

    /// Push a value. Returns the evicted value if buffer was full.
    pub fn push(&mut self, val: f64) -> Option<f64> {
        let evicted = if self.len == self.cap {
            Some(self.data[self.head])
        } else {
            None
        };

        if self.len < self.cap {
            self.data.push(val);
            self.len += 1;
        } else {
            self.data[self.head] = val;
            self.head += 1;
            if self.head >= self.cap {
                self.head = 0;
            }
        }

        evicted
    }

    /// Get value at logical index `i` (0 = oldest).
    #[inline]
    pub fn get(&self, i: usize) -> Option<f64> {
        if i < self.len {
            let idx = self.head + i;
            Some(self.data[if idx >= self.cap { idx - self.cap } else { idx }])
        } else {
            None
        }
    }

    #[inline]
    pub fn last(&self) -> Option<f64> {
        if self.len == 0 {
            None
        } else {
            let idx = self.head + self.len - 1;
            Some(self.data[if idx >= self.cap { idx - self.cap } else { idx }])
        }
    }

    pub fn clear(&mut self) {
        self.data.clear();
        self.head = 0;
        self.len = 0;
    }

    pub fn iter(&self) -> RingVecIter<'_> {
        RingVecIter { buf: self, pos: 0 }
    }
}

impl<'a> IntoIterator for &'a RingVec {
    type Item = f64;
    type IntoIter = RingVecIter<'a>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct RingVecIter<'a> {
    buf: &'a RingVec,
    pos: usize,
}

impl Iterator for RingVecIter<'_> {
    type Item = f64;
    fn next(&mut self) -> Option<f64> {
        let v = self.buf.get(self.pos);
        self.pos += 1;
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── RingVec tests ──

    #[test]
    fn ringvec_new_empty() {
        let rv = RingVec::new(16);
        assert_eq!(rv.len(), 0);
        assert_eq!(rv.capacity(), 16);
        assert!(rv.is_empty());
        assert!(!rv.is_full());
    }

    #[test]
    fn ringvec_push_until_full() {
        let mut rv = RingVec::new(3);
        assert_eq!(rv.push(1.0), None);
        assert_eq!(rv.push(2.0), None);
        assert_eq!(rv.push(3.0), None);
        assert!(rv.is_full());
        assert_eq!(rv.len(), 3);
    }

    #[test]
    fn ringvec_evict_on_overflow() {
        let mut rv = RingVec::new(3);
        rv.push(1.0);
        rv.push(2.0);
        rv.push(3.0);
        assert_eq!(rv.push(4.0), Some(1.0));
        assert_eq!(rv.len(), 3);
    }

    #[test]
    fn ringvec_get_logical_order() {
        let mut rv = RingVec::new(3);
        rv.push(10.0);
        rv.push(20.0);
        rv.push(30.0);
        rv.push(40.0); // evicts 10.0
        assert_eq!(rv.get(0), Some(20.0));
        assert_eq!(rv.get(1), Some(30.0));
        assert_eq!(rv.get(2), Some(40.0));
        assert_eq!(rv.get(3), None);
    }

    #[test]
    fn ringvec_last() {
        let mut rv = RingVec::new(3);
        assert_eq!(rv.last(), None);
        rv.push(1.0);
        assert_eq!(rv.last(), Some(1.0));
        rv.push(2.0);
        assert_eq!(rv.last(), Some(2.0));
        rv.push(3.0);
        rv.push(4.0); // evicts 1
        assert_eq!(rv.last(), Some(4.0));
    }

    #[test]
    fn ringvec_iter() {
        let mut rv = RingVec::new(3);
        rv.push(1.0);
        rv.push(2.0);
        rv.push(3.0);
        let vals: Vec<f64> = rv.iter().collect();
        assert_eq!(vals, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn ringvec_iter_after_eviction() {
        let mut rv = RingVec::new(3);
        rv.push(1.0);
        rv.push(2.0);
        rv.push(3.0);
        rv.push(4.0); // evicts 1
        let vals: Vec<f64> = rv.iter().collect();
        assert_eq!(vals, vec![2.0, 3.0, 4.0]);
    }

    #[test]
    fn ringvec_iter_sum() {
        let mut rv = RingVec::new(4);
        for v in [1.0, 2.0, 3.0, 4.0] {
            rv.push(v);
        }
        assert!((rv.iter().sum::<f64>() - 10.0).abs() < 1e-10);
    }

    #[test]
    fn ringvec_zero_capacity_does_not_panic() {
        let mut rv = RingVec::new(0);
        assert_eq!(rv.capacity(), 1);
        assert_eq!(rv.push(42.0), None);
        assert_eq!(rv.get(0), Some(42.0));
    }

    #[test]
    fn ringvec_clear() {
        let mut rv = RingVec::new(3);
        rv.push(1.0);
        rv.push(2.0);
        rv.clear();
        assert_eq!(rv.len(), 0);
        assert!(rv.is_empty());
        assert_eq!(rv.get(0), None);
    }

    #[test]
    fn ringvec_clear_reuse() {
        let mut rv = RingVec::new(3);
        rv.push(1.0);
        rv.push(2.0);
        rv.push(3.0);
        rv.clear();
        rv.push(10.0);
        assert_eq!(rv.get(0), Some(10.0));
        assert_eq!(rv.len(), 1);
    }

    // ── RingBuffer tests ──

    #[test]
    fn ringbuffer_new_empty() {
        let rb: RingBuffer<f64, 4> = RingBuffer::new();
        assert_eq!(rb.len(), 0);
        assert_eq!(rb.capacity(), 4);
        assert!(rb.is_empty());
        assert!(!rb.is_full());
    }

    #[test]
    fn ringbuffer_push_until_full() {
        let mut rb: RingBuffer<f64, 4> = RingBuffer::new();
        rb.push(1.0);
        rb.push(2.0);
        rb.push(3.0);
        rb.push(4.0);
        assert!(rb.is_full());
        assert_eq!(rb.len(), 4);
    }

    #[test]
    fn ringbuffer_evict_on_overflow() {
        let mut rb: RingBuffer<f64, 4> = RingBuffer::new();
        rb.push(1.0);
        rb.push(2.0);
        rb.push(3.0);
        rb.push(4.0);
        rb.push(5.0);
        assert_eq!(rb.len(), 4);
        assert_eq!(rb.get(0), Some(&2.0));
        assert_eq!(rb.get(1), Some(&3.0));
        assert_eq!(rb.get(2), Some(&4.0));
        assert_eq!(rb.get(3), Some(&5.0));
    }

    #[test]
    fn ringbuffer_get_logical_order() {
        let mut rb: RingBuffer<f64, 4> = RingBuffer::new();
        rb.push(10.0);
        rb.push(20.0);
        rb.push(30.0);
        rb.push(40.0);
        rb.push(50.0); // evicts 10.0
        assert_eq!(rb.get(0), Some(&20.0));
        assert_eq!(rb.get(1), Some(&30.0));
        assert_eq!(rb.get(2), Some(&40.0));
        assert_eq!(rb.get(3), Some(&50.0));
        assert_eq!(rb.get(4), None);
    }

    #[test]
    fn ringbuffer_last() {
        let mut rb: RingBuffer<f64, 4> = RingBuffer::new();
        assert_eq!(rb.last(), None);
        rb.push(1.0);
        assert_eq!(rb.last(), Some(&1.0));
        rb.push(2.0);
        assert_eq!(rb.last(), Some(&2.0));
        rb.push(3.0);
        rb.push(4.0);
        rb.push(5.0); // evicts 1
        assert_eq!(rb.last(), Some(&5.0));
    }

    #[test]
    fn ringbuffer_iter() {
        let mut rb: RingBuffer<f64, 4> = RingBuffer::new();
        rb.push(1.0);
        rb.push(2.0);
        rb.push(3.0);
        rb.push(4.0);
        let vals: Vec<&f64> = rb.iter().collect();
        assert_eq!(vals, vec![&1.0, &2.0, &3.0, &4.0]);
    }

    #[test]
    fn ringbuffer_iter_after_eviction() {
        let mut rb: RingBuffer<f64, 4> = RingBuffer::new();
        rb.push(1.0);
        rb.push(2.0);
        rb.push(3.0);
        rb.push(4.0);
        rb.push(5.0); // evicts 1
        let vals: Vec<&f64> = rb.iter().collect();
        assert_eq!(vals, vec![&2.0, &3.0, &4.0, &5.0]);
    }

    #[test]
    fn ringbuffer_clear() {
        let mut rb: RingBuffer<f64, 4> = RingBuffer::new();
        rb.push(1.0);
        rb.push(2.0);
        rb.clear();
        assert_eq!(rb.len(), 0);
        assert!(rb.is_empty());
        assert_eq!(rb.get(0), None);
    }

    #[test]
    fn ringbuffer_clear_reuse() {
        let mut rb: RingBuffer<f64, 4> = RingBuffer::new();
        rb.push(1.0);
        rb.push(2.0);
        rb.push(3.0);
        rb.clear();
        rb.push(10.0);
        assert_eq!(rb.get(0), Some(&10.0));
        assert_eq!(rb.len(), 1);
    }

    #[test]
    fn ringbuffer_get_out_of_bounds() {
        let rb: RingBuffer<f64, 4> = RingBuffer::new();
        assert_eq!(rb.get(0), None);
        assert_eq!(rb.get(100), None);
    }
}
