/// Rolling Window Engine — O(1) push/evict with online statistics.
///
/// Wraps `RingVec` with mean, variance, stddev, min, max, sum.
/// Allocation-free hot path after initial capacity allocation.
use quince_core::ring::RingVec;

#[derive(Debug, Clone)]
pub struct RollingWindow {
    buf: RingVec,
    sum: f64,
    sum_sq: f64,
    min: f64,
    max: f64,
}

impl RollingWindow {
    pub fn new(capacity: usize) -> Self {
        RollingWindow {
            buf: RingVec::new(capacity),
            sum: 0.0,
            sum_sq: 0.0,
            min: f64::MAX,
            max: f64::MIN,
        }
    }

    pub fn len(&self) -> usize { self.buf.len() }

    pub fn capacity(&self) -> usize { self.buf.capacity() }

    pub fn is_empty(&self) -> bool { self.buf.is_empty() }

    pub fn is_full(&self) -> bool { self.buf.is_full() }

    /// Push a value. Updates running statistics in O(1).
    pub fn push(&mut self, val: f64) {
        let evicted = self.buf.push(val);

        self.sum += val;
        self.sum_sq += val * val;

        if let Some(old) = evicted {
            self.sum -= old;
            self.sum_sq -= old * old;

            let min_was_evicted = old == self.min;
            let max_was_evicted = old == self.max;

            if min_was_evicted {
                self.min = if val <= old { val } else { self.recompute_min() };
            }
            if max_was_evicted {
                self.max = if val >= old { val } else { self.recompute_max() };
            }

            if !min_was_evicted && val < self.min { self.min = val; }
            if !max_was_evicted && val > self.max { self.max = val; }
        } else {
            if val < self.min { self.min = val; }
            if val > self.max { self.max = val; }
        }
    }

    fn recompute_min(&self) -> f64 {
        let mut m = f64::MAX;
        for i in 0..self.buf.len() {
            if let Some(v) = self.buf.get(i) {
                if v < m { m = v; }
            }
        }
        m
    }

    fn recompute_max(&self) -> f64 {
        let mut m = f64::MIN;
        for i in 0..self.buf.len() {
            if let Some(v) = self.buf.get(i) {
                if v > m { m = v; }
            }
        }
        m
    }

    pub fn mean(&self) -> f64 {
        if self.buf.is_empty() { return 0.0; }
        self.sum / self.buf.len() as f64
    }

    /// Population variance (not sample variance).
    pub fn variance(&self) -> f64 {
        if self.buf.len() < 2 { return 0.0; }
        let n = self.buf.len() as f64;
        (self.sum_sq - self.sum * self.sum / n) / n
    }

    pub fn stddev(&self) -> f64 {
        self.variance().sqrt()
    }

    pub fn sum(&self) -> f64 { self.sum }

    pub fn min(&self) -> f64 {
        if self.buf.is_empty() { 0.0 } else { self.min }
    }

    pub fn max(&self) -> f64 {
        if self.buf.is_empty() { 0.0 } else { self.max }
    }

    pub fn last(&self) -> Option<f64> { self.buf.last() }

    pub fn get(&self, i: usize) -> Option<f64> { self.buf.get(i) }

    pub fn clear(&mut self) {
        self.buf.clear();
        self.sum = 0.0;
        self.sum_sq = 0.0;
        self.min = f64::MAX;
        self.max = f64::MIN;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_empty() {
        let w = RollingWindow::new(16);
        assert_eq!(w.len(), 0);
        assert_eq!(w.capacity(), 16);
        assert!(w.is_empty());
        assert!(!w.is_full());
        assert_eq!(w.mean(), 0.0);
        assert_eq!(w.min(), 0.0);
        assert_eq!(w.max(), 0.0);
        assert_eq!(w.sum(), 0.0);
    }

    #[test]
    fn push_single_value() {
        let mut w = RollingWindow::new(4);
        w.push(42.0);
        assert_eq!(w.len(), 1);
        assert!((w.mean() - 42.0).abs() < 1e-10);
        assert!((w.sum() - 42.0).abs() < 1e-10);
        assert!((w.min() - 42.0).abs() < 1e-10);
        assert!((w.max() - 42.0).abs() < 1e-10);
    }

    #[test]
    fn push_multiple_values() {
        let mut w = RollingWindow::new(4);
        w.push(10.0); w.push(20.0); w.push(30.0);
        assert_eq!(w.len(), 3);
        assert!((w.mean() - 20.0).abs() < 1e-10);
        assert!((w.sum() - 60.0).abs() < 1e-10);
        assert!((w.min() - 10.0).abs() < 1e-10);
        assert!((w.max() - 30.0).abs() < 1e-10);
    }

    #[test]
    fn eviction_maintains_correct_stats() {
        let mut w = RollingWindow::new(3);
        w.push(10.0); w.push(20.0); w.push(30.0);
        // Window: [10, 20, 30]
        assert!((w.mean() - 20.0).abs() < 1e-10);
        assert!((w.sum() - 60.0).abs() < 1e-10);

        // Evict 10, push 40 → Window: [20, 30, 40]
        w.push(40.0);
        assert_eq!(w.len(), 3);
        assert!((w.mean() - 30.0).abs() < 1e-10);
        assert!((w.sum() - 90.0).abs() < 1e-10);
        assert!((w.min() - 20.0).abs() < 1e-10);
        assert!((w.max() - 40.0).abs() < 1e-10);
    }

    #[test]
    fn stddev_of_constant_values_is_zero() {
        let mut w = RollingWindow::new(5);
        for _ in 0..5 { w.push(100.0); }
        assert!((w.variance() - 0.0).abs() < 1e-10);
        assert!((w.stddev() - 0.0).abs() < 1e-10);
    }

    #[test]
    fn stddev_of_known_values() {
        let mut w = RollingWindow::new(4);
        w.push(2.0); w.push(4.0); w.push(4.0); w.push(4.0);
        // mean = 3.5, variance = ((2-3.5)^2 + (4-3.5)^2 + (4-3.5)^2 + (4-3.5)^2) / 4
        // = (2.25 + 0.25 + 0.25 + 0.25) / 4 = 3.0 / 4 = 0.75
        let expected_var: f64 = 0.75;
        let expected_std = expected_var.sqrt();
        assert!((w.variance() - expected_var).abs() < 1e-10);
        assert!((w.stddev() - expected_std).abs() < 1e-10);
    }

    #[test]
    fn min_max_after_eviction() {
        let mut w = RollingWindow::new(3);
        w.push(5.0); w.push(1.0); w.push(10.0);
        assert!((w.min() - 1.0).abs() < 1e-10);
        assert!((w.max() - 10.0).abs() < 1e-10);

        // Evict 5 (not extreme), push 3 → Window: [1, 10, 3]
        w.push(3.0);
        assert!((w.min() - 1.0).abs() < 1e-10);
        assert!((w.max() - 10.0).abs() < 1e-10);

        // Evict 1 (the min), push 2 → Window: [10, 3, 2]
        w.push(2.0);
        assert!((w.min() - 2.0).abs() < 1e-10);
        assert!((w.max() - 10.0).abs() < 1e-10);
    }

    #[test]
    fn clear_resets_all_stats() {
        let mut w = RollingWindow::new(4);
        w.push(10.0); w.push(20.0);
        w.clear();
        assert_eq!(w.len(), 0);
        assert_eq!(w.mean(), 0.0);
        assert_eq!(w.sum(), 0.0);
        assert_eq!(w.min(), 0.0);
        assert_eq!(w.max(), 0.0);
    }

    #[test]
    fn get_returns_logical_order() {
        let mut w = RollingWindow::new(3);
        w.push(10.0); w.push(20.0); w.push(30.0);
        w.push(40.0); // evicts 10
        assert!((w.get(0).unwrap() - 20.0).abs() < 1e-10);
        assert!((w.get(1).unwrap() - 30.0).abs() < 1e-10);
        assert!((w.get(2).unwrap() - 40.0).abs() < 1e-10);
        assert_eq!(w.get(3), None);
    }

    #[test]
    fn last_returns_newest_value() {
        let mut w = RollingWindow::new(3);
        assert_eq!(w.last(), None);
        w.push(1.0);
        assert!((w.last().unwrap() - 1.0).abs() < 1e-10);
        w.push(2.0);
        assert!((w.last().unwrap() - 2.0).abs() < 1e-10);
    }

    #[test]
    fn variance_single_value_is_zero() {
        let mut w = RollingWindow::new(5);
        w.push(99.0);
        assert_eq!(w.variance(), 0.0);
    }

    #[test]
    fn rolling_mean_matches_sequential_expectation() {
        let mut w = RollingWindow::new(4);
        for v in &[10.0, 20.0, 30.0, 40.0] {
            w.push(*v);
        }
        assert!((w.mean() - 25.0).abs() < 1e-10);
        w.push(50.0); // evicts 10
        // Window: [20, 30, 40, 50], mean = 35
        assert!((w.mean() - 35.0).abs() < 1e-10);
        w.push(60.0); // evicts 20
        // Window: [30, 40, 50, 60], mean = 45
        assert!((w.mean() - 45.0).abs() < 1e-10);
    }

    #[test]
    fn capacity_one_window() {
        let mut w = RollingWindow::new(1);
        w.push(100.0);
        assert_eq!(w.len(), 1);
        assert!((w.mean() - 100.0).abs() < 1e-10);
        w.push(200.0); // evicts 100
        assert_eq!(w.len(), 1);
        assert!((w.mean() - 200.0).abs() < 1e-10);
        assert!((w.min() - 200.0).abs() < 1e-10);
        assert!((w.max() - 200.0).abs() < 1e-10);
    }

    #[test]
    fn negative_values() {
        let mut w = RollingWindow::new(3);
        w.push(-5.0); w.push(-10.0); w.push(-3.0);
        assert!((w.min() - (-10.0)).abs() < 1e-10);
        assert!((w.max() - (-3.0)).abs() < 1e-10);
        assert!((w.mean() - (-6.0)).abs() < 1e-10);
        assert!((w.sum() - (-18.0)).abs() < 1e-10);
    }
}
