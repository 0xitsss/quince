use crate::Candle;
use quince_core::ring::RingVec;

pub struct Adx {
    period: usize,
    tr_buffer: RingVec,
    plus_dm_buffer: RingVec,
    minus_dm_buffer: RingVec,
    prev_candle: Option<Candle>,
    count: usize,
    tr_smooth: Option<f64>,
    plus_di: Option<f64>,
    minus_di: Option<f64>,
    adx_ema: Option<f64>,
}

impl Adx {
    pub fn new(period: usize) -> Self {
        assert!(period > 1, "ADX period must be > 1");
        Self {
            period, tr_buffer: RingVec::new(period),
            plus_dm_buffer: RingVec::new(period),
            minus_dm_buffer: RingVec::new(period),
            prev_candle: None, count: 0,
            tr_smooth: None, plus_di: None, minus_di: None, adx_ema: None,
        }
    }

    pub fn update(&mut self, candle: &Candle) -> Option<f64> {
        if let Some(prev) = &self.prev_candle {
            let high_diff = candle.high - prev.high;
            let low_diff = prev.low - candle.low;
            let plus_dm = if high_diff > low_diff && high_diff > 0.0 { high_diff } else { 0.0 };
            let minus_dm = if low_diff > high_diff && low_diff > 0.0 { low_diff } else { 0.0 };
            let tr = ((candle.high - candle.low).max((candle.high - prev.close).abs()).max((candle.low - prev.close).abs())).max(0.0);

            if self.count < self.period {
                self.tr_buffer.push(tr);
                self.plus_dm_buffer.push(plus_dm);
                self.minus_dm_buffer.push(minus_dm);
                self.count += 1;
                if self.count == self.period {
                    let tr_sum: f64 = self.tr_buffer.iter().sum();
                    let pdm_sum: f64 = self.plus_dm_buffer.iter().sum();
                    let mdm_sum: f64 = self.minus_dm_buffer.iter().sum();
                    self.tr_smooth = Some(tr_sum);
                    let pdi = if tr_sum == 0.0 { 0.0 } else { pdm_sum / tr_sum * 100.0 };
                    let mdi = if tr_sum == 0.0 { 0.0 } else { mdm_sum / tr_sum * 100.0 };
                    self.plus_di = Some(pdi);
                    self.minus_di = Some(mdi);
                    let dx = if (pdi + mdi) == 0.0 { 0.0 } else { (pdi - mdi).abs() / (pdi + mdi) * 100.0 };
                    self.adx_ema = Some(dx);
                    return self.adx_ema;
                }
            } else {
                let k = 1.0 / self.period as f64;
                self.tr_smooth = Some(self.tr_smooth.unwrap() * (1.0 - k) + tr * k);
                let ts = self.tr_smooth.unwrap();
                self.plus_di = Some((self.plus_di.unwrap() * (self.period as f64 - 1.0) + plus_dm * 100.0 / if ts == 0.0 { 1.0 } else { ts }) / self.period as f64);
                self.minus_di = Some((self.minus_di.unwrap() * (self.period as f64 - 1.0) + minus_dm * 100.0 / if ts == 0.0 { 1.0 } else { ts }) / self.period as f64);
                let pdi = self.plus_di.unwrap();
                let mdi = self.minus_di.unwrap();
                let dx = if (pdi + mdi) == 0.0 { 0.0 } else { (pdi - mdi).abs() / (pdi + mdi) * 100.0 };
                self.adx_ema = Some(self.adx_ema.unwrap() * (1.0 - k) + dx * k);
                return self.adx_ema;
            }
        }
        self.prev_candle = Some(*candle);
        None
    }

    pub fn reset(&mut self) {
        self.tr_buffer.clear();
        self.plus_dm_buffer.clear();
        self.minus_dm_buffer.clear();
        self.prev_candle = None;
        self.count = 0;
        self.tr_smooth = None;
        self.plus_di = None;
        self.minus_di = None;
        self.adx_ema = None;
    }

    pub fn di(&self) -> Option<(f64, f64)> {
        self.plus_di.zip(self.minus_di)
    }
}

pub struct BidAskImbalance;

impl BidAskImbalance {
    pub fn update(bid_volume: f64, ask_volume: f64) -> f64 {
        let total = bid_volume + ask_volume;
        if total == 0.0 { return 0.0 }
        (bid_volume - ask_volume) / total * 100.0
    }
}

pub struct DomDepth;

impl DomDepth {
    pub fn update(bids: &[(f64, f64)], asks: &[(f64, f64)], mark_price: f64, percent: f64) -> (f64, f64) {
        let range = mark_price * percent / 100.0;
        let bid_depth: f64 = bids.iter().filter(|&&(p, _)| p >= mark_price - range).map(|&(_, q)| q).sum();
        let ask_depth: f64 = asks.iter().filter(|&&(p, _)| p <= mark_price + range).map(|&(_, q)| q).sum();
        (bid_depth, ask_depth)
    }

    pub fn imbalance(bids: &[(f64, f64)], asks: &[(f64, f64)], mark_price: f64, percent: f64) -> f64 {
        let (bd, ad) = Self::update(bids, asks, mark_price, percent);
        let total = bd + ad;
        if total == 0.0 { return 0.0 }
        (bd - ad) / total * 100.0
    }
}

pub struct ZScore {
    period: usize,
    buffer: RingVec,
}

impl ZScore {
    pub fn new(period: usize) -> Self {
        assert!(period > 1, "ZScore period must be > 1");
        Self { period, buffer: RingVec::new(period) }
    }

    pub fn update(&mut self, value: f64) -> Option<f64> {
        self.buffer.push(value);
        if self.buffer.len() == self.period {
            let mean: f64 = self.buffer.iter().sum::<f64>() / self.period as f64;
            let variance: f64 = self.buffer.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / self.period as f64;
            let stddev = variance.sqrt();
            if stddev == 0.0 { return Some(0.0) }
            Some((value - mean) / stddev)
        } else {
            None
        }
    }

    pub fn classify(z: f64) -> &'static str {
        let az = z.abs();
        if az < 0.5 { "tiny" }
        else if az < 1.0 { "small" }
        else if az < 2.0 { "normal" }
        else if az < 3.0 { "large" }
        else { "huge" }
    }

    pub fn reset(&mut self) { self.buffer.clear(); }
}

pub struct NetOpenInterest;

impl NetOpenInterest {
    pub fn update(volume_delta: f64, oi_delta: f64) -> NetOiOutput {
        let (taker_long, taker_short) = if volume_delta > 0.0 {
            if oi_delta > 0.0 { (oi_delta, 0.0) } else { (0.0, oi_delta) }
        } else {
            if oi_delta > 0.0 { (0.0, oi_delta) } else { (oi_delta, 0.0) }
        };
        NetOiOutput { taker_long: taker_long.abs(), taker_short: taker_short.abs(), volume_delta, oi_delta }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetOiOutput {
    pub taker_long: f64,
    pub taker_short: f64,
    pub volume_delta: f64,
    pub oi_delta: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adx_basic() {
        let mut adx = Adx::new(5);
        for i in 0..6 {
            let c = Candle::new(i as f64, (i+1) as f64, (i-1).max(0) as f64, i as f64, 100.0);
            adx.update(&c);
        }
        assert!(adx.update(&Candle::new(6.0, 7.0, 5.0, 6.0, 100.0)).is_some());
    }

    #[test]
    fn adx_not_enough_data() {
        let mut adx = Adx::new(5);
        for i in 0..5 {
            assert_eq!(adx.update(&Candle::new(i as f64, (i+1) as f64, i as f64, i as f64, 100.0)), None);
        }
    }

    #[test]
    fn adx_reset() {
        let mut adx = Adx::new(3);
        for i in 0..5 { adx.update(&Candle::new(i as f64, (i+1) as f64, i as f64, i as f64, 100.0)); }
        adx.reset();
        assert_eq!(adx.update(&Candle::new(1.0, 2.0, 1.0, 1.0, 100.0)), None);
    }

    #[test]
    fn adx_constant_prices() {
        let mut adx = Adx::new(3);
        for _ in 0..6 { adx.update(&Candle::new(10.0, 10.0, 10.0, 10.0, 100.0)); }
        let v = adx.update(&Candle::new(10.0, 10.0, 10.0, 10.0, 100.0));
        assert!(v.is_some());
    }

    #[test]
    fn adx_up_trend_di() {
        let mut adx = Adx::new(5);
        for i in 0..10 { adx.update(&Candle::new(i as f64, (i+1) as f64, i as f64, i as f64, 100.0)); }
        adx.update(&Candle::new(10.0, 11.0, 10.0, 10.0, 100.0));
        let (pdi, mdi) = adx.di().unwrap();
        assert!(pdi > mdi);
    }

    #[test]
    #[should_panic]
    fn adx_period_one_panics() { Adx::new(1); }

    #[test]
    fn imbalance_basic() {
        let v = BidAskImbalance::update(100.0, 60.0);
        assert!((v - 25.0).abs() < 1e-10);
    }

    #[test]
    fn imbalance_zero_total() {
        assert!((BidAskImbalance::update(0.0, 0.0) - 0.0).abs() < 1e-10);
    }

    #[test]
    fn dom_depth_basic() {
        let bids = vec![(101.0, 10.0), (100.0, 20.0), (99.0, 30.0)];
        let asks = vec![(102.0, 15.0), (103.0, 25.0)];
        let (bd, ad) = DomDepth::update(&bids, &asks, 100.0, 2.0);
        assert!((bd - 60.0).abs() < 1e-10);
        assert!((ad - 15.0).abs() < 1e-10);
    }

    #[test]
    fn dom_depth_empty() {
        let (bd, ad) = DomDepth::update(&[], &[], 100.0, 2.0);
        assert!((bd - 0.0).abs() < 1e-10);
        assert!((ad - 0.0).abs() < 1e-10);
    }

    #[test]
    fn dom_imbalance() {
        let bids = vec![(100.0, 100.0)];
        let asks = vec![(101.0, 50.0)];
        let imb = DomDepth::imbalance(&bids, &asks, 100.0, 2.0);
        assert!((imb - 33.33333).abs() < 0.001);
    }

    #[test]
    fn zscore_basic() {
        let mut z = ZScore::new(5);
        for _ in 0..4 { z.update(10.0); }
        let v = z.update(10.0).unwrap();
        assert!((v - 0.0).abs() < 1e-10);
    }

    #[test]
    fn zscore_outlier() {
        let mut z = ZScore::new(5);
        z.update(10.0); z.update(10.0); z.update(10.0); z.update(10.0);
        let v = z.update(100.0).unwrap();
        assert!(v > 1.0);
    }

    #[test]
    fn zscore_constant() {
        let mut z = ZScore::new(3);
        z.update(5.0); z.update(5.0);
        assert!((z.update(5.0).unwrap() - 0.0).abs() < 1e-10);
    }

    #[test]
    fn zscore_classify() {
        assert_eq!(ZScore::classify(0.3), "tiny");
        assert_eq!(ZScore::classify(0.7), "small");
        assert_eq!(ZScore::classify(1.5), "normal");
        assert_eq!(ZScore::classify(2.5), "large");
        assert_eq!(ZScore::classify(3.5), "huge");
    }

    #[test]
    fn zscore_reset() {
        let mut z = ZScore::new(3);
        z.update(1.0); z.update(2.0);
        z.reset();
        assert_eq!(z.update(10.0), None);
    }

    #[test]
    #[should_panic]
    fn zscore_period_one_panics() { ZScore::new(1); }

    #[test]
    fn net_oi_both_positive() {
        let r = NetOpenInterest::update(100.0, 50.0);
        assert!((r.taker_long - 50.0).abs() < 1e-10);
        assert!((r.taker_short - 0.0).abs() < 1e-10);
    }

    #[test]
    fn net_oi_vol_neg_oi_pos() {
        let r = NetOpenInterest::update(-100.0, 50.0);
        assert!((r.taker_long - 0.0).abs() < 1e-10);
        assert!((r.taker_short - 50.0).abs() < 1e-10);
    }

    #[test]
    fn net_oi_vol_pos_oi_neg() {
        let r = NetOpenInterest::update(100.0, -50.0);
        assert!((r.taker_long - 0.0).abs() < 1e-10);
        assert!((r.taker_short - 50.0).abs() < 1e-10);
    }

    #[test]
    fn net_oi_both_negative() {
        let r = NetOpenInterest::update(-100.0, -50.0);
        assert!((r.taker_long - 50.0).abs() < 1e-10);
        assert!((r.taker_short - 0.0).abs() < 1e-10);
    }
}
