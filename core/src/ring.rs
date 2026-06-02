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

    pub fn len(&self) -> usize { self.len }

    pub const fn capacity(&self) -> usize { N }

    pub fn is_empty(&self) -> bool { self.len == 0 }

    pub fn is_full(&self) -> bool { self.len == N }

    fn idx(&self, i: usize) -> usize {
        (self.head + i) % N
    }

    pub fn push(&mut self, val: T) {
        if self.len < N {
            self.buf[self.idx(self.len)] = MaybeUninit::new(val);
            self.len += 1;
        } else {
            let slot = self.head;
            unsafe { ptr::drop_in_place(self.buf[slot].as_mut_ptr()); }
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
        if self.len == 0 { None }
        else { Some(unsafe { self.buf[self.idx(self.len - 1)].assume_init_ref() }) }
    }

    pub fn clear(&mut self) {
        for i in 0..self.len {
            unsafe { ptr::drop_in_place(self.buf[self.idx(i)].as_mut_ptr()); }
        }
        self.head = 0;
        self.len = 0;
    }

    pub fn iter(&self) -> RingIter<'_, T, N> {
        RingIter { buf: self, pos: 0 }
    }
}

impl<T, const N: usize> Drop for RingBuffer<T, N> {
    fn drop(&mut self) {
        for i in 0..self.len {
            unsafe { ptr::drop_in_place(self.buf[self.idx(i)].as_mut_ptr()); }
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
