use quince_core::ring::RingVec;

pub struct Sma {
    period: usize,
    buffer: RingVec,
    sum: f64,
}

impl Sma {
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "SMA period must be > 0");
        Self {
            period,
            buffer: RingVec::new(period),
            sum: 0.0,
        }
    }

    pub fn update(&mut self, value: f64) -> Option<f64> {
        if let Some(evicted) = self.buffer.push(value) {
            self.sum -= evicted;
        }
        self.sum += value;
        if self.buffer.len() == self.period {
            Some(self.sum / self.period as f64)
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
        self.sum = 0.0;
    }

    pub fn is_ready(&self) -> bool {
        self.buffer.len() == self.period
    }
}

pub struct Ema {
    multiplier: f64,
    current: Option<f64>,
}

impl Ema {
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "EMA period must be > 0");
        Self {
            multiplier: 2.0 / (period + 1) as f64,
            current: None,
        }
    }

    pub fn update(&mut self, value: f64) -> f64 {
        self.current = Some(match self.current {
            Some(prev) => (value - prev) * self.multiplier + prev,
            None => value,
        });
        self.current.unwrap()
    }

    pub fn reset(&mut self) {
        self.current = None;
    }
    pub fn value(&self) -> Option<f64> {
        self.current
    }
}

pub struct Wma {
    period: usize,
    buffer: RingVec,
    denominator: f64,
}

impl Wma {
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "WMA period must be > 0");
        let denominator = (period * (period + 1) / 2) as f64;
        Self {
            period,
            buffer: RingVec::new(period),
            denominator,
        }
    }

    pub fn update(&mut self, value: f64) -> Option<f64> {
        self.buffer.push(value);
        if self.buffer.len() == self.period {
            let weighted: f64 = self
                .buffer
                .iter()
                .enumerate()
                .map(|(i, v)| v * (i + 1) as f64)
                .sum();
            Some(weighted / self.denominator)
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
    }
    pub fn is_ready(&self) -> bool {
        self.buffer.len() == self.period
    }
}

pub struct Vwma {
    period: usize,
    price_buffer: RingVec,
    vol_buffer: RingVec,
    pv_sum: f64,
    v_sum: f64,
}

impl Vwma {
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "VWMA period must be > 0");
        Self {
            period,
            price_buffer: RingVec::new(period),
            vol_buffer: RingVec::new(period),
            pv_sum: 0.0,
            v_sum: 0.0,
        }
    }

    pub fn update(&mut self, price: f64, volume: f64) -> Option<f64> {
        let evicted_p = self.price_buffer.push(price);
        let evicted_v = self.vol_buffer.push(volume);

        self.pv_sum += price * volume;
        self.v_sum += volume;

        if let (Some(ep), Some(ev)) = (evicted_p, evicted_v) {
            self.pv_sum -= ep * ev;
            self.v_sum -= ev;
        }

        if self.price_buffer.len() == self.period {
            if self.v_sum == 0.0 {
                return None;
            }
            Some(self.pv_sum / self.v_sum)
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.price_buffer.clear();
        self.vol_buffer.clear();
        self.pv_sum = 0.0;
        self.v_sum = 0.0;
    }
    pub fn is_ready(&self) -> bool {
        self.price_buffer.len() == self.period
    }
}

pub struct Lsma {
    period: usize,
    buffer: RingVec,
    sum_x: f64,
    sum_x2: f64,
}

impl Lsma {
    pub fn new(period: usize) -> Self {
        assert!(period > 1, "LSMA period must be > 1");
        let n = period as f64;
        let sum_x = n * (n - 1.0) / 2.0;
        let sum_x2 = (n - 1.0) * n * (2.0 * n - 1.0) / 6.0;
        Self {
            period,
            buffer: RingVec::new(period),
            sum_x,
            sum_x2,
        }
    }

    pub fn update(&mut self, value: f64) -> Option<f64> {
        self.buffer.push(value);
        if self.buffer.len() == self.period {
            let n = self.period as f64;
            let sum_y: f64 = self.buffer.iter().sum();
            let sum_xy: f64 = self
                .buffer
                .iter()
                .enumerate()
                .map(|(i, v)| v * i as f64)
                .sum();
            let slope =
                (n * sum_xy - self.sum_x * sum_y) / (n * self.sum_x2 - self.sum_x * self.sum_x);
            let intercept = (sum_y - slope * self.sum_x) / n;
            Some(intercept + slope * (n - 1.0))
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.buffer.clear();
    }
    pub fn is_ready(&self) -> bool {
        self.buffer.len() == self.period
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sma_returns_none_without_enough_data() {
        let mut sma = Sma::new(3);
        assert_eq!(sma.update(1.0), None);
        assert_eq!(sma.update(2.0), None);
    }

    #[test]
    fn sma_returns_value_when_ready() {
        let mut sma = Sma::new(3);
        sma.update(1.0);
        sma.update(2.0);
        assert_eq!(sma.update(3.0), Some(2.0));
    }

    #[test]
    fn sma_sliding_window() {
        let mut sma = Sma::new(3);
        sma.update(1.0);
        sma.update(2.0);
        sma.update(3.0);
        assert_eq!(sma.update(4.0), Some(3.0));
        assert_eq!(sma.update(5.0), Some(4.0));
    }

    #[test]
    fn sma_period_one() {
        let mut sma = Sma::new(1);
        assert_eq!(sma.update(42.0), Some(42.0));
    }

    #[test]
    fn sma_reset() {
        let mut sma = Sma::new(3);
        sma.update(1.0);
        sma.update(2.0);
        sma.reset();
        assert_eq!(sma.update(10.0), None);
        assert!(!sma.is_ready());
    }

    #[test]
    fn sma_zero_values() {
        let mut sma = Sma::new(3);
        sma.update(0.0);
        sma.update(0.0);
        assert_eq!(sma.update(0.0), Some(0.0));
    }

    #[test]
    fn sma_negative_values() {
        let mut sma = Sma::new(3);
        sma.update(-1.0);
        sma.update(-2.0);
        assert_eq!(sma.update(-3.0), Some(-2.0));
    }

    #[test]
    #[should_panic]
    fn sma_zero_period_panics() {
        Sma::new(0);
    }

    #[test]
    fn ema_first_value_equals_input() {
        let mut ema = Ema::new(3);
        assert_eq!(ema.update(10.0), 10.0);
    }

    #[test]
    fn ema_convergence() {
        let mut ema = Ema::new(3);
        ema.update(10.0);
        let v = ema.update(10.0);
        assert!((v - 10.0).abs() < 1e-10);
    }

    #[test]
    fn ema_tracks_trend() {
        let mut ema = Ema::new(3);
        ema.update(10.0);
        let v = ema.update(20.0);
        assert!(v > 10.0 && v < 20.0);
    }

    #[test]
    fn ema_after_many_identical_values() {
        let mut ema = Ema::new(5);
        for _ in 0..100 {
            ema.update(42.0);
        }
        assert!((ema.value().unwrap() - 42.0).abs() < 1e-10);
    }

    #[test]
    fn ema_reset() {
        let mut ema = Ema::new(3);
        ema.update(10.0);
        ema.reset();
        assert_eq!(ema.value(), None);
        assert_eq!(ema.update(99.0), 99.0);
    }

    #[test]
    fn ema_negative_input() {
        let mut ema = Ema::new(3);
        ema.update(-5.0);
        let v = ema.update(-10.0);
        assert!(v < -5.0);
    }

    #[test]
    #[should_panic]
    fn ema_zero_period_panics() {
        Ema::new(0);
    }

    #[test]
    fn wma_known_values() {
        let mut wma = Wma::new(3);
        wma.update(1.0);
        wma.update(2.0);
        let v = wma.update(3.0);
        let expected = (1.0 * 1.0 + 2.0 * 2.0 + 3.0 * 3.0) / 6.0;
        assert!((v.unwrap() - expected).abs() < 1e-10);
    }

    #[test]
    fn wma_sliding() {
        let mut wma = Wma::new(2);
        wma.update(1.0);
        wma.update(2.0);
        let v = wma.update(3.0);
        let expected = (2.0 * 1.0 + 3.0 * 2.0) / 3.0;
        assert!((v.unwrap() - expected).abs() < 1e-10);
    }

    #[test]
    fn wma_not_enough_data() {
        let mut wma = Wma::new(5);
        for i in 1..=4 {
            assert_eq!(wma.update(i as f64), None);
        }
        assert!(wma.update(5.0).is_some());
    }

    #[test]
    fn wma_reset() {
        let mut wma = Wma::new(3);
        wma.update(1.0);
        wma.update(2.0);
        wma.reset();
        assert_eq!(wma.update(10.0), None);
    }

    #[test]
    fn wma_period_one() {
        let mut wma = Wma::new(1);
        assert_eq!(wma.update(99.0), Some(99.0));
    }

    #[test]
    #[should_panic]
    fn wma_zero_period_panics() {
        Wma::new(0);
    }

    #[test]
    fn vwma_known_values() {
        let mut vwma = Vwma::new(3);
        vwma.update(10.0, 100.0);
        vwma.update(20.0, 200.0);
        let v = vwma.update(30.0, 300.0);
        let expected = (10.0 * 100.0 + 20.0 * 200.0 + 30.0 * 300.0) / (100.0 + 200.0 + 300.0);
        assert!((v.unwrap() - expected).abs() < 1e-10);
    }

    #[test]
    fn vwma_not_enough_data() {
        let mut vwma = Vwma::new(3);
        assert_eq!(vwma.update(1.0, 1.0), None);
        assert_eq!(vwma.update(2.0, 1.0), None);
    }

    #[test]
    fn vwma_sliding() {
        let mut vwma = Vwma::new(2);
        vwma.update(10.0, 100.0);
        vwma.update(20.0, 200.0);
        let v = vwma.update(30.0, 300.0);
        assert!((v.unwrap() - (20.0 * 200.0 + 30.0 * 300.0) / 500.0).abs() < 1e-10);
    }

    #[test]
    fn vwma_zero_volume_returns_none() {
        let mut vwma = Vwma::new(2);
        vwma.update(10.0, 0.0);
        vwma.update(20.0, 0.0);
        assert_eq!(vwma.update(30.0, 0.0), None);
    }

    #[test]
    fn vwma_reset() {
        let mut vwma = Vwma::new(3);
        vwma.update(1.0, 1.0);
        vwma.update(2.0, 1.0);
        vwma.reset();
        assert_eq!(vwma.update(10.0, 10.0), None);
    }

    #[test]
    #[should_panic]
    fn vwma_zero_period_panics() {
        Vwma::new(0);
    }

    #[test]
    fn lsma_known_values() {
        let mut lsma = Lsma::new(3);
        lsma.update(1.0);
        lsma.update(2.0);
        let v = lsma.update(3.0);
        assert!((v.unwrap() - 3.0).abs() < 1e-10);
    }

    #[test]
    fn lsma_linear_trend() {
        let mut lsma = Lsma::new(4);
        lsma.update(1.0);
        lsma.update(2.0);
        lsma.update(3.0);
        let v = lsma.update(4.0);
        assert!((v.unwrap() - 4.0).abs() < 1e-10);
    }

    #[test]
    fn lsma_not_enough_data() {
        let mut lsma = Lsma::new(5);
        for i in 1..=4 {
            assert_eq!(lsma.update(i as f64), None);
        }
        assert!(lsma.update(5.0).is_some());
    }

    #[test]
    fn lsma_reset() {
        let mut lsma = Lsma::new(3);
        lsma.update(1.0);
        lsma.update(2.0);
        lsma.reset();
        assert_eq!(lsma.update(10.0), None);
    }

    #[test]
    fn lsma_constant_values() {
        let mut lsma = Lsma::new(3);
        lsma.update(5.0);
        lsma.update(5.0);
        let v = lsma.update(5.0);
        assert!((v.unwrap() - 5.0).abs() < 1e-10);
    }

    #[test]
    fn lsma_negative_values() {
        let mut lsma = Lsma::new(3);
        lsma.update(-1.0);
        lsma.update(-2.0);
        let v = lsma.update(-3.0);
        assert!((v.unwrap() - (-3.0)).abs() < 1e-10);
    }

    #[test]
    #[should_panic]
    fn lsma_period_one_panics() {
        Lsma::new(1);
    }

    #[test]
    fn lsma_sliding() {
        let mut lsma = Lsma::new(3);
        lsma.update(1.0);
        lsma.update(2.0);
        lsma.update(3.0);
        let v = lsma.update(10.0);
        assert!(v.unwrap() > 5.0);
    }
}
