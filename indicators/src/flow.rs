use crate::Candle;
use std::collections::VecDeque;

pub struct Mfi {
    period: usize,
    typical_prev: Option<f64>,
    pos_flow: VecDeque<f64>,
    neg_flow: VecDeque<f64>,
    count: usize,
}

impl Mfi {
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "MFI period must be > 0");
        Self { period, typical_prev: None, pos_flow: VecDeque::with_capacity(period), neg_flow: VecDeque::with_capacity(period), count: 0 }
    }

    pub fn update(&mut self, candle: &Candle) -> Option<f64> {
        let tp = candle.typical();
        let money_flow = tp * candle.volume;

        if let Some(prev_tp) = self.typical_prev {
            if self.count < self.period {
                if tp > prev_tp { self.pos_flow.push_back(money_flow); self.neg_flow.push_back(0.0); }
                else if tp < prev_tp { self.pos_flow.push_back(0.0); self.neg_flow.push_back(money_flow); }
                else { self.pos_flow.push_back(0.0); self.neg_flow.push_back(0.0); }
                self.count += 1;
            } else {
                if tp > prev_tp {
                    let _ = self.pos_flow.pop_front().map(|_v| self.pos_flow.push_back(money_flow));
                    let _ = self.neg_flow.pop_front().map(|_v| self.neg_flow.push_back(0.0));
                } else if tp < prev_tp {
                    let _ = self.pos_flow.pop_front().map(|_v| self.pos_flow.push_back(0.0));
                    let _ = self.neg_flow.pop_front().map(|_v| self.neg_flow.push_back(money_flow));
                } else {
                    let _ = self.pos_flow.pop_front().map(|_v| self.pos_flow.push_back(0.0));
                    let _ = self.neg_flow.pop_front().map(|_v| self.neg_flow.push_back(0.0));
                }
            }

            if self.count >= self.period {
                let pos_sum: f64 = self.pos_flow.iter().sum();
                let neg_sum: f64 = self.neg_flow.iter().sum();
                if pos_sum == 0.0 && neg_sum == 0.0 { return Some(50.0) }
                if neg_sum == 0.0 { return Some(100.0) }
                let mfr = pos_sum / neg_sum;
                Some(100.0 - 100.0 / (1.0 + mfr))
            } else {
                None
            }
        } else {
            self.typical_prev = Some(tp);
            None
        }
    }

    pub fn reset(&mut self) { self.typical_prev = None; self.pos_flow.clear(); self.neg_flow.clear(); self.count = 0; }
}

pub struct VolumeDelta;

impl VolumeDelta {
    pub fn update(buy_volume: f64, sell_volume: f64) -> f64 {
        buy_volume - sell_volume
    }
}

pub struct Cvd {
    cumulative: f64,
}

impl Cvd {
    pub fn new() -> Self { Self { cumulative: 0.0 } }

    pub fn update(&mut self, _trade_price: f64, trade_qty: f64, is_buyer_aggressive: bool) -> f64 {
        if is_buyer_aggressive {
            self.cumulative += trade_qty;
        } else {
            self.cumulative -= trade_qty;
        }
        self.cumulative
    }

    pub fn reset(&mut self) { self.cumulative = 0.0; }
    pub fn value(&self) -> f64 { self.cumulative }
}

pub struct Obv {
    obv: f64,
    prev_close: Option<f64>,
}

impl Obv {
    pub fn new() -> Self { Self { obv: 0.0, prev_close: None } }

    pub fn update(&mut self, close: f64, volume: f64) -> f64 {
        if let Some(prev) = self.prev_close {
            if close > prev { self.obv += volume; }
            else if close < prev { self.obv -= volume; }
        }
        self.prev_close = Some(close);
        self.obv
    }

    pub fn reset(&mut self) { self.obv = 0.0; self.prev_close = None; }
    pub fn value(&self) -> f64 { self.obv }
}

pub struct AccDist {
    ad: f64,
}

impl AccDist {
    pub fn new() -> Self { Self { ad: 0.0 } }

    pub fn update(&mut self, candle: &Candle) -> f64 {
        let clv = if candle.high == candle.low {
            0.0
        } else {
            ((candle.close - candle.low) - (candle.high - candle.close)) / (candle.high - candle.low) * candle.volume
        };
        self.ad += clv;
        self.ad
    }

    pub fn reset(&mut self) { self.ad = 0.0; }
    pub fn value(&self) -> f64 { self.ad }
}

pub struct Pmdi {
    value: f64,
    prev_data: Option<f64>,
}

impl Pmdi {
    pub fn new() -> Self { Self { value: 0.0, prev_data: None } }

    pub fn update(&mut self, data: f64, close: f64) -> f64 {
        if let Some(prev_data) = self.prev_data {
            if data > prev_data {
                let growth = (close + prev_data) / prev_data;
                self.value = self.value + growth * self.value.max(1.0);
            }
        } else {
            self.value = close;
        }
        self.prev_data = Some(data);
        self.value
    }

    pub fn reset(&mut self) { self.value = 0.0; self.prev_data = None; }
    pub fn value(&self) -> f64 { self.value }
}

pub struct Nmdi {
    value: f64,
    prev_data: Option<f64>,
}

impl Nmdi {
    pub fn new() -> Self { Self { value: 0.0, prev_data: None } }

    pub fn update(&mut self, data: f64, close: f64) -> f64 {
        if let Some(prev_data) = self.prev_data {
            if data < prev_data {
                let growth = (close + prev_data) / prev_data;
                self.value = self.value + growth * self.value.max(1.0);
            }
        } else {
            self.value = close;
        }
        self.prev_data = Some(data);
        self.value
    }

    pub fn reset(&mut self) { self.value = 0.0; self.prev_data = None; }
    pub fn value(&self) -> f64 { self.value }
}

pub struct AverageTradeSize;

impl AverageTradeSize {
    pub fn update(volume: f64, trade_count: f64) -> f64 {
        if trade_count == 0.0 { return 0.0 }
        volume / trade_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mfi_basic() {
        let mut mfi = Mfi::new(3);
        assert_eq!(mfi.update(&Candle::new(10.0, 11.0, 9.0, 10.0, 100.0)), None);
        assert_eq!(mfi.update(&Candle::new(11.0, 12.0, 10.0, 11.0, 200.0)), None);
        assert_eq!(mfi.update(&Candle::new(12.0, 13.0, 11.0, 12.0, 300.0)), None);
        assert!(mfi.update(&Candle::new(13.0, 14.0, 12.0, 13.0, 400.0)).is_some());
    }

    #[test]
    fn mfi_constant_price_gives_50() {
        let mut mfi = Mfi::new(3);
        mfi.update(&Candle::new(10.0, 10.0, 10.0, 10.0, 100.0));
        mfi.update(&Candle::new(10.0, 10.0, 10.0, 10.0, 100.0));
        mfi.update(&Candle::new(10.0, 10.0, 10.0, 10.0, 100.0));
        let v = mfi.update(&Candle::new(10.0, 10.0, 10.0, 10.0, 100.0));
        assert!((v.unwrap() - 50.0).abs() < 1e-10);
    }

    #[test]
    fn mfi_all_positive_gives_100() {
        let mut mfi = Mfi::new(3);
        mfi.update(&Candle::new(10.0, 10.0, 10.0, 10.0, 100.0));
        mfi.update(&Candle::new(11.0, 11.0, 11.0, 11.0, 200.0));
        mfi.update(&Candle::new(12.0, 12.0, 12.0, 12.0, 300.0));
        let v = mfi.update(&Candle::new(13.0, 13.0, 13.0, 13.0, 400.0));
        assert!((v.unwrap() - 100.0).abs() < 1e-10);
    }

    #[test]
    fn mfi_reset() {
        let mut mfi = Mfi::new(3);
        for _ in 0..5 { mfi.update(&Candle::new(1.0, 1.0, 1.0, 1.0, 1.0)); }
        mfi.reset();
        assert_eq!(mfi.update(&Candle::new(10.0, 10.0, 10.0, 10.0, 100.0)), None);
    }

    #[test]
    #[should_panic]
    fn mfi_zero_period_panics() { Mfi::new(0); }

    #[test]
    fn volume_delta_basic() {
        assert!((VolumeDelta::update(100.0, 60.0) - 40.0).abs() < 1e-10);
        assert!((VolumeDelta::update(50.0, 80.0) + 30.0).abs() < 1e-10);
    }

    #[test]
    fn cvd_basic() {
        let mut cvd = Cvd::new();
        assert!((cvd.update(100.0, 10.0, true) - 10.0).abs() < 1e-10);
        assert!((cvd.update(100.0, 5.0, false) - 5.0).abs() < 1e-10);
    }

    #[test]
    fn cvd_reset() {
        let mut cvd = Cvd::new();
        cvd.update(100.0, 100.0, true);
        cvd.reset();
        assert!((cvd.update(100.0, 1.0, false) + 1.0).abs() < 1e-10);
    }

    #[test]
    fn obv_basic() {
        let mut obv = Obv::new();
        obv.update(10.0, 100.0);
        assert!((obv.update(12.0, 50.0) - 50.0).abs() < 1e-10);
        assert!((obv.update(11.0, 30.0) - 20.0).abs() < 1e-10);
    }

    #[test]
    fn obv_no_change() {
        let mut obv = Obv::new();
        obv.update(10.0, 100.0);
        assert!((obv.update(10.0, 999.0) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn obv_reset() {
        let mut obv = Obv::new();
        obv.update(1.0, 1.0);
        obv.reset();
        assert!((obv.value() - 0.0).abs() < 1e-10);
    }

    #[test]
    fn accdist_basic() {
        let mut ad = AccDist::new();
        let v = ad.update(&Candle::new(10.0, 15.0, 5.0, 12.0, 1000.0));
        assert!(v > 0.0);
    }

    #[test]
    fn accdist_high_equals_low() {
        let mut ad = AccDist::new();
        let v = ad.update(&Candle::new(10.0, 10.0, 10.0, 10.0, 1000.0));
        assert!((v - 0.0).abs() < 1e-10);
    }

    #[test]
    fn accdist_reset() {
        let mut ad = AccDist::new();
        ad.update(&Candle::new(1.0, 2.0, 1.0, 1.5, 100.0));
        ad.reset();
        assert!((ad.value() - 0.0).abs() < 1e-10);
    }

    #[test]
    fn pmdi_first_value_sets_base() {
        let mut pmdi = Pmdi::new();
        assert!((pmdi.update(100.0, 10.0) - 10.0).abs() < 1e-10);
    }

    #[test]
    fn pmdi_increases_when_data_up() {
        let mut pmdi = Pmdi::new();
        pmdi.update(100.0, 10.0);
        let v = pmdi.update(110.0, 12.0);
        assert!(v > 10.0);
    }

    #[test]
    fn pmdi_does_not_change_when_data_down() {
        let mut pmdi = Pmdi::new();
        pmdi.update(100.0, 10.0);
        let v = pmdi.update(90.0, 9.0);
        assert!((v - 10.0).abs() < 1e-10);
    }

    #[test]
    fn pmdi_reset() {
        let mut pmdi = Pmdi::new();
        pmdi.update(100.0, 10.0); pmdi.update(110.0, 12.0);
        pmdi.reset();
        assert!((pmdi.update(100.0, 10.0) - 10.0).abs() < 1e-10);
    }

    #[test]
    fn nmdi_first_value_sets_base() {
        let mut nmdi = Nmdi::new();
        assert!((nmdi.update(100.0, 10.0) - 10.0).abs() < 1e-10);
    }

    #[test]
    fn nmdi_increases_when_data_down() {
        let mut nmdi = Nmdi::new();
        nmdi.update(100.0, 10.0);
        let v = nmdi.update(90.0, 9.0);
        assert!(v > 10.0);
    }

    #[test]
    fn nmdi_does_not_change_when_data_up() {
        let mut nmdi = Nmdi::new();
        nmdi.update(100.0, 10.0);
        let v = nmdi.update(110.0, 12.0);
        assert!((v - 10.0).abs() < 1e-10);
    }

    #[test]
    fn nmdi_reset() {
        let mut nmdi = Nmdi::new();
        nmdi.update(100.0, 10.0); nmdi.update(90.0, 9.0);
        nmdi.reset();
        assert!((nmdi.update(100.0, 10.0) - 10.0).abs() < 1e-10);
    }

    #[test]
    fn average_trade_size_basic() {
        assert!((AverageTradeSize::update(1000.0, 10.0) - 100.0).abs() < 1e-10);
    }

    #[test]
    fn average_trade_size_zero_count() {
        assert!((AverageTradeSize::update(1000.0, 0.0) - 0.0).abs() < 1e-10);
    }
}
