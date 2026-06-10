// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Debug-only ring buffer for strategy log messages.
//!
//! Stores the most recent `max` log entries, dropping oldest when full.
//! Only compiled in `debug_assertions` builds.

use std::collections::VecDeque;

/// Ring buffer for strategy log messages.
///
/// Drops oldest entries when `max` capacity is reached.
#[derive(Debug, Clone)]
pub struct LogBuffer {
    entries: VecDeque<String>,
    max: usize,
}

impl LogBuffer {
    pub fn new(max: usize) -> Self {
        Self {
            entries: VecDeque::with_capacity(if max > 0 { max.min(4096) } else { 0 }),
            max,
        }
    }

    pub fn push(&mut self, msg: String) {
        if self.max == 0 {
            return;
        }
        if self.entries.len() >= self.max {
            self.entries.pop_front();
        }
        self.entries.push_back(msg);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn drain(&mut self) -> impl Iterator<Item = String> + '_ {
        self.entries.drain(..)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_buffer_empty() {
        let buf = LogBuffer::new(10);
        assert!(buf.is_empty());
        assert_eq!(buf.len(), 0);
    }

    #[test]
    fn push_increments_len() {
        let mut buf = LogBuffer::new(10);
        buf.push("hello".into());
        assert_eq!(buf.len(), 1);
    }

    #[test]
    fn push_evicts_oldest() {
        let mut buf = LogBuffer::new(3);
        buf.push("a".into());
        buf.push("b".into());
        buf.push("c".into());
        buf.push("d".into());
        assert_eq!(buf.len(), 3);
        let msgs: Vec<String> = buf.drain().collect();
        assert_eq!(msgs, vec!["b", "c", "d"]);
    }

    #[test]
    fn drain_empties_buffer() {
        let mut buf = LogBuffer::new(5);
        buf.push("x".into());
        buf.push("y".into());
        let _: Vec<_> = buf.drain().collect();
        assert!(buf.is_empty());
    }

    #[test]
    fn drain_returns_all() {
        let mut buf = LogBuffer::new(10);
        buf.push("first".into());
        buf.push("second".into());
        let msgs: Vec<String> = buf.drain().collect();
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn max_caps_size() {
        let mut buf = LogBuffer::new(2);
        buf.push("1".into());
        buf.push("2".into());
        buf.push("3".into());
        buf.push("4".into());
        assert_eq!(buf.len(), 2);
    }

    #[test]
    fn zero_max_never_stores() {
        let mut buf = LogBuffer::new(0);
        buf.push("test".into());
        assert!(buf.is_empty());
    }
}
