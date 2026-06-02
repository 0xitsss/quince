use std::collections::VecDeque;

pub struct Rsi {
    period: usize,
    gains: VecDeque<f64>,
    losses: VecDeque<f64>,
    avg_gain: Option<f64>,
    avg_loss: Option<f64>,
    prev: Option<f64>,
    count: usize,
}

impl Rsi {
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "RSI period must be > 0");
        Self { period, gains: VecDeque::with_capacity(period), losses: VecDeque::with_capacity(period), avg_gain: None, avg_loss: None, prev: None, count: 0 }
    }

    pub fn update(&mut self, price: f64) -> Option<f64> {
        if let Some(prev) = self.prev {
            let diff = price - prev;
            let gain = diff.max(0.0);
            let loss = (-diff).max(0.0);

            if self.count < self.period {
                self.gains.push_back(gain);
                self.losses.push_back(loss);
                self.count += 1;
                if self.count == self.period {
                    self.avg_gain = Some(self.gains.iter().sum::<f64>() / self.period as f64);
                    self.avg_loss = Some(self.losses.iter().sum::<f64>() / self.period as f64);
                }
            } else {
                let a_gain = self.avg_gain.unwrap();
                let a_loss = self.avg_loss.unwrap();
                self.avg_gain = Some((a_gain * (self.period as f64 - 1.0) + gain) / self.period as f64);
                self.avg_loss = Some((a_loss * (self.period as f64 - 1.0) + loss) / self.period as f64);
            }
        }
        self.prev = Some(price);

        if let (Some(ag), Some(al)) = (self.avg_gain, self.avg_loss) {
            if al == 0.0 {
                return if ag == 0.0 { Some(50.0) } else { Some(100.0) };
            }
            let rs = ag / al;
            Some(100.0 - 100.0 / (1.0 + rs))
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.gains.clear(); self.losses.clear();
        self.avg_gain = None; self.avg_loss = None; self.prev = None;
        self.count = 0;
    }
}

pub struct Macd {
    fast_ema: super::ma::Ema,
    slow_ema: super::ma::Ema,
    signal_ema: super::ma::Ema,
}

impl Macd {
    pub fn new(fast: usize, slow: usize, signal: usize) -> Self {
        assert!(fast > 0 && slow > 0 && signal > 0);
        assert!(fast < slow, "fast period must be < slow period");
        Self {
            fast_ema: super::ma::Ema::new(fast),
            slow_ema: super::ma::Ema::new(slow),
            signal_ema: super::ma::Ema::new(signal),
        }
    }

    pub fn update(&mut self, price: f64) -> Option<MacdOutput> {
        self.fast_ema.update(price);
        self.slow_ema.update(price);
        if let (Some(f), Some(s)) = (self.fast_ema.value(), self.slow_ema.value()) {
            let macd_line = f - s;
            let signal_line = self.signal_ema.update(macd_line);
            if self.signal_ema.value().is_some() {
                Some(MacdOutput { macd_line, signal_line, histogram: macd_line - signal_line })
            } else {
                // signal ema needs warmup: pass macd values until signal period is reached
                None
            }
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.fast_ema.reset(); self.slow_ema.reset(); self.signal_ema.reset();
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MacdOutput {
    pub macd_line: f64,
    pub signal_line: f64,
    pub histogram: f64,
}

pub struct Cci {
    period: usize,
    typical_buffer: VecDeque<f64>,
    constant: f64,
}

impl Cci {
    pub fn new(period: usize, constant: f64) -> Self {
        assert!(period > 0, "CCI period must be > 0");
        Self { period, typical_buffer: VecDeque::with_capacity(period), constant }
    }

    pub fn update(&mut self, high: f64, low: f64, close: f64) -> Option<f64> {
        let tp = (high + low + close) / 3.0;
        self.typical_buffer.push_back(tp);
        if self.typical_buffer.len() > self.period {
            self.typical_buffer.pop_front();
        }
        if self.typical_buffer.len() == self.period {
            let sma_tp: f64 = self.typical_buffer.iter().sum::<f64>() / self.period as f64;
            let mad: f64 = self.typical_buffer.iter().map(|&v| (v - sma_tp).abs()).sum::<f64>() / self.period as f64;
            if mad == 0.0 { return Some(0.0) }
            Some((tp - sma_tp) / (self.constant * mad))
        } else {
            None
        }
    }

    pub fn reset(&mut self) { self.typical_buffer.clear(); }
}

pub struct Roc {
    period: usize,
    buffer: VecDeque<f64>,
}

impl Roc {
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "ROC period must be > 0");
        Self { period, buffer: VecDeque::with_capacity(period + 1) }
    }

    pub fn update(&mut self, price: f64) -> Option<f64> {
        self.buffer.push_back(price);
        if self.buffer.len() > self.period + 1 {
            self.buffer.pop_front();
        }
        if self.buffer.len() == self.period + 1 {
            let prev = self.buffer.front().unwrap();
            if *prev == 0.0 { return None }
            Some((price - prev) / prev * 100.0)
        } else {
            None
        }
    }

    pub fn reset(&mut self) { self.buffer.clear(); }
}

pub struct Stochastic {
    period: usize,
    high_buffer: VecDeque<f64>,
    low_buffer: VecDeque<f64>,
}

impl Stochastic {
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "Stochastic period must be > 0");
        Self { period, high_buffer: VecDeque::with_capacity(period), low_buffer: VecDeque::with_capacity(period) }
    }

    pub fn update(&mut self, high: f64, low: f64, close: f64) -> Option<f64> {
        self.high_buffer.push_back(high);
        self.low_buffer.push_back(low);
        if self.high_buffer.len() > self.period {
            self.high_buffer.pop_front();
            self.low_buffer.pop_front();
        }
        if self.high_buffer.len() == self.period {
            let highest = self.high_buffer.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            let lowest = self.low_buffer.iter().cloned().fold(f64::INFINITY, f64::min);
            if highest == lowest { return Some(50.0) }
            Some((close - lowest) / (highest - lowest) * 100.0)
        } else {
            None
        }
    }

    pub fn reset(&mut self) { self.high_buffer.clear(); self.low_buffer.clear(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rsi_returns_none_without_enough_data() {
        let mut rsi = Rsi::new(14);
        assert_eq!(rsi.update(10.0), None);
    }

    #[test]
    fn rsi_constant_prices_give_50() {
        let mut rsi = Rsi::new(3);
        for _ in 0..10 { rsi.update(42.0); }
        assert!((rsi.update(42.0).unwrap() - 50.0).abs() < 1e-10);
    }

    #[test]
    fn rsi_all_gains_gives_100() {
        let mut rsi = Rsi::new(3);
        rsi.update(10.0); rsi.update(11.0); rsi.update(12.0);
        let v = rsi.update(13.0);
        assert!((v.unwrap() - 100.0).abs() < 1e-10);
    }

    #[test]
    fn rsi_all_losses_gives_0() {
        let mut rsi = Rsi::new(3);
        rsi.update(10.0); rsi.update(9.0); rsi.update(8.0);
        let v = rsi.update(7.0);
        assert!((v.unwrap() - 0.0).abs() < 1e-10);
    }

    #[test]
    fn rsi_mixed() {
        let mut rsi = Rsi::new(3);
        rsi.update(10.0); rsi.update(12.0); rsi.update(11.0);
        let v = rsi.update(13.0);
        assert!(v.unwrap() > 50.0 && v.unwrap() < 100.0);
    }

    #[test]
    fn rsi_reset() {
        let mut rsi = Rsi::new(3);
        rsi.update(10.0); rsi.update(11.0); rsi.update(12.0);
        rsi.reset();
        assert_eq!(rsi.update(99.0), None);
    }

    #[test]
    #[should_panic]
    fn rsi_zero_period_panics() { Rsi::new(0); }

    #[test]
    fn macd_basic() {
        let mut macd = Macd::new(3, 6, 2);
        for _ in 0..10 { macd.update(10.0); }
        let out = macd.update(10.0);
        assert!(out.is_some());
        let o = out.unwrap();
        assert!((o.macd_line).abs() < 1e-6);
        assert!((o.histogram).abs() < 1e-6);
    }

    #[test]
    fn macd_upward_trend() {
        let mut macd = Macd::new(3, 6, 2);
        for i in 1..=20 { macd.update(i as f64); }
        let o = macd.update(21.0).unwrap();
        assert!(o.macd_line > 0.0);
    }

    #[test]
    fn macd_reset() {
        let mut macd = Macd::new(3, 6, 2);
        macd.update(10.0);
        macd.reset();
        for _ in 0..10 { macd.update(10.0); }
        assert!(macd.update(10.0).is_some());
    }

    #[test]
    #[should_panic]
    fn macd_fast_not_less_than_slow() { Macd::new(10, 5, 2); }

    #[test]
    fn cci_known_values() {
        let mut cci = Cci::new(5, 0.015);
        for i in 1..=5 { cci.update((i+10) as f64, (i+9) as f64, (i+10) as f64); }
        let v = cci.update(16.0, 14.0, 15.0);
        assert!(v.is_some());
    }

    #[test]
    fn cci_not_enough_data() {
        let mut cci = Cci::new(5, 0.015);
        for i in 1..=4 { assert_eq!(cci.update(i as f64, i as f64, i as f64), None); }
    }

    #[test]
    fn cci_constant_prices() {
        let mut cci = Cci::new(3, 0.015);
        for _ in 0..4 { cci.update(10.0, 10.0, 10.0); }
        assert!((cci.update(10.0, 10.0, 10.0).unwrap() - 0.0).abs() < 1e-10);
    }

    #[test]
    fn cci_reset() {
        let mut cci = Cci::new(3, 0.015);
        for _ in 0..4 { cci.update(1.0, 1.0, 1.0); }
        cci.reset();
        assert_eq!(cci.update(1.0, 1.0, 1.0), None);
    }

    #[test]
    #[should_panic]
    fn cci_zero_period_panics() { Cci::new(0, 0.015); }

    #[test]
    fn roc_positive_change() {
        let mut roc = Roc::new(3);
        roc.update(100.0); roc.update(101.0); roc.update(102.0);
        let v = roc.update(110.0);
        assert!((v.unwrap() - 10.0).abs() < 1e-10);
    }

    #[test]
    fn roc_negative_change() {
        let mut roc = Roc::new(3);
        roc.update(100.0); roc.update(99.0); roc.update(98.0);
        let v = roc.update(90.0);
        assert!((v.unwrap() + 10.0).abs() < 1e-10);
    }

    #[test]
    fn roc_no_change() {
        let mut roc = Roc::new(3);
        roc.update(100.0); roc.update(100.0); roc.update(100.0);
        assert!((roc.update(100.0).unwrap() - 0.0).abs() < 1e-10);
    }

    #[test]
    fn roc_not_enough_data() {
        let mut roc = Roc::new(5);
        for i in 1..=5 { assert_eq!(roc.update(i as f64), None); }
        assert!(roc.update(6.0).is_some());
    }

    #[test]
    fn roc_zero_prev_returns_none() {
        let mut roc = Roc::new(2);
        roc.update(0.0); roc.update(0.0);
        assert_eq!(roc.update(10.0), None);
    }

    #[test]
    fn roc_reset() {
        let mut roc = Roc::new(3);
        for i in 1..=5 { roc.update(i as f64); }
        roc.reset();
        assert_eq!(roc.update(10.0), None);
    }

    #[test]
    #[should_panic]
    fn roc_zero_period_panics() { Roc::new(0); }

    #[test]
    fn stoch_known_values() {
        let mut stoch = Stochastic::new(3);
        stoch.update(10.0, 8.0, 9.0); stoch.update(12.0, 9.0, 11.0);
        let v = stoch.update(11.0, 10.0, 10.5);
        let expected = (10.5 - 8.0) / (12.0 - 8.0) * 100.0;
        assert!((v.unwrap() - expected).abs() < 1e-10);
    }

    #[test]
    fn stoch_not_enough_data() {
        let mut stoch = Stochastic::new(5);
        for _ in 0..4 { assert_eq!(stoch.update(1.0, 1.0, 1.0), None); }
    }

    #[test]
    fn stoch_high_equals_low_gives_50() {
        let mut stoch = Stochastic::new(3);
        for _ in 0..5 { stoch.update(10.0, 10.0, 10.0); }
        assert!((stoch.update(10.0, 10.0, 10.0).unwrap() - 50.0).abs() < 1e-10);
    }

    #[test]
    fn stoch_reset() {
        let mut stoch = Stochastic::new(3);
        for _ in 0..5 { stoch.update(1.0, 1.0, 1.0); }
        stoch.reset();
        assert_eq!(stoch.update(1.0, 1.0, 1.0), None);
    }

    #[test]
    #[should_panic]
    fn stoch_zero_period_panics() { Stochastic::new(0); }
}
