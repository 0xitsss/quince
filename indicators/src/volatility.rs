// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Volatility indicators.
//! Provides [`TrueRange`], [`Atr`] (Average True Range), [`BollingerBands`],
//! and [`KeltnerChannel`] for measuring and visualizing market volatility.

use quince_core::ring::RingVec;

pub struct TrueRange;

impl TrueRange {
    pub fn update(prev_close: f64, high: f64, low: f64) -> f64 {
        let hl = high - low;
        let hc = (high - prev_close).abs();
        let lc = (low - prev_close).abs();
        hl.max(hc).max(lc)
    }
}

pub struct Atr {
    period: usize,
    atr: Option<f64>,
    prev_close: Option<f64>,
    count: usize,
    initial_tr: RingVec,
}

impl Atr {
    pub fn new(period: usize) -> Self {
        assert!(period > 0, "ATR period must be > 0");
        Self {
            period,
            atr: None,
            prev_close: None,
            count: 0,
            initial_tr: RingVec::new(period),
        }
    }

    pub fn update(&mut self, high: f64, low: f64, close: f64) -> Option<f64> {
        let tr = match self.prev_close {
            Some(pc) => TrueRange::update(pc, high, low),
            None => high - low,
        };
        self.prev_close = Some(close);

        if self.count < self.period {
            self.initial_tr.push(tr);
            self.count += 1;
            if self.count == self.period {
                self.atr = Some(self.initial_tr.iter().sum::<f64>() / self.period as f64);
            }
            None
        } else {
            self.atr =
                Some((self.atr.unwrap() * (self.period as f64 - 1.0) + tr) / self.period as f64);
            self.atr
        }
    }

    pub fn reset(&mut self) {
        self.atr = None;
        self.prev_close = None;
        self.count = 0;
        self.initial_tr.clear();
    }
}

pub struct BollingerBands {
    period: usize,
    multiplier: f64,
    sma: super::ma::Sma,
    buffer: RingVec,
}

impl BollingerBands {
    pub fn new(period: usize, multiplier: f64) -> Self {
        assert!(period > 0 && multiplier > 0.0);
        Self {
            period,
            multiplier,
            sma: super::ma::Sma::new(period),
            buffer: RingVec::new(period),
        }
    }

    pub fn update(&mut self, price: f64) -> Option<BollingerOutput> {
        self.buffer.push(price);
        let middle = self.sma.update(price)?;
        if self.buffer.len() == self.period {
            let variance: f64 = self
                .buffer
                .iter()
                .map(|v| (v - middle).powi(2))
                .sum::<f64>()
                / self.period as f64;
            let stddev = variance.sqrt();
            Some(BollingerOutput {
                middle,
                upper: middle + self.multiplier * stddev,
                lower: middle - self.multiplier * stddev,
                bandwidth: if middle == 0.0 {
                    0.0
                } else {
                    stddev * 2.0 * self.multiplier / middle * 100.0
                },
            })
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.sma.reset();
        self.buffer.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BollingerOutput {
    pub middle: f64,
    pub upper: f64,
    pub lower: f64,
    pub bandwidth: f64,
}

pub struct KeltnerChannel {
    multiplier: f64,
    ema: super::ma::Ema,
    atr: Atr,
}

impl KeltnerChannel {
    pub fn new(period: usize, multiplier: f64) -> Self {
        assert!(period > 0 && multiplier > 0.0);
        Self {
            multiplier,
            ema: super::ma::Ema::new(period),
            atr: Atr::new(period),
        }
    }

    pub fn update(&mut self, high: f64, low: f64, close: f64) -> Option<KeltnerOutput> {
        let middle = self.ema.update(close);
        if self.atr.update(high, low, close).is_some() {
            let atr_val = self.atr.atr.unwrap();
            Some(KeltnerOutput {
                middle,
                upper: middle + self.multiplier * atr_val,
                lower: middle - self.multiplier * atr_val,
            })
        } else {
            None
        }
    }

    pub fn reset(&mut self) {
        self.ema.reset();
        self.atr.reset();
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KeltnerOutput {
    pub middle: f64,
    pub upper: f64,
    pub lower: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn true_range_hl() {
        assert!((TrueRange::update(10.0, 15.0, 12.0) - 5.0).abs() < 1e-10);
    }

    #[test]
    fn true_range_hc() {
        assert!((TrueRange::update(10.0, 14.0, 11.0) - 4.0).abs() < 1e-10);
    }

    #[test]
    fn true_range_lc() {
        assert!((TrueRange::update(15.0, 14.0, 13.0) - 2.0).abs() < 1e-10);
    }

    #[test]
    fn atr_basic() {
        let mut atr = Atr::new(3);
        assert_eq!(atr.update(10.0, 9.0, 9.5), None);
        assert_eq!(atr.update(10.5, 9.5, 10.0), None);
        assert_eq!(atr.update(11.0, 10.0, 10.5), None);
        assert!(atr.update(11.5, 10.5, 11.0).is_some());
    }

    #[test]
    fn atr_sliding() {
        let mut atr = Atr::new(3);
        atr.update(10.0, 9.0, 9.5);
        atr.update(11.0, 10.0, 10.5);
        atr.update(12.0, 11.0, 11.5);
        assert!(atr.update(13.0, 12.0, 12.5).is_some());
    }

    #[test]
    fn atr_constant_range() {
        let mut atr = Atr::new(3);
        for _ in 0..3 {
            atr.update(10.0, 9.0, 9.5);
        }
        let v = atr.update(10.0, 9.0, 9.5).unwrap();
        assert!((v - 1.0).abs() < 1e-10);
    }

    #[test]
    fn atr_reset() {
        let mut atr = Atr::new(3);
        for _ in 0..5 {
            atr.update(1.0, 1.0, 1.0);
        }
        atr.reset();
        assert_eq!(atr.update(1.0, 1.0, 1.0), None);
    }

    #[test]
    #[should_panic]
    fn atr_zero_period_panics() {
        Atr::new(0);
    }

    #[test]
    fn bb_known_values() {
        let mut bb = BollingerBands::new(3, 2.0);
        assert_eq!(bb.update(10.0), None);
        assert_eq!(bb.update(10.0), None);
        let o = bb.update(10.0).unwrap();
        assert!((o.middle - 10.0).abs() < 1e-10);
        assert!((o.upper - 10.0).abs() < 1e-10);
        assert!((o.lower - 10.0).abs() < 1e-10);
    }

    #[test]
    fn bb_spread() {
        let mut bb = BollingerBands::new(3, 2.0);
        bb.update(10.0);
        bb.update(12.0);
        let o = bb.update(14.0).unwrap();
        assert!(o.upper > o.middle);
        assert!(o.lower < o.middle);
    }

    #[test]
    fn bb_bandwidth() {
        let mut bb = BollingerBands::new(3, 2.0);
        bb.update(10.0);
        bb.update(10.0);
        let o = bb.update(10.0).unwrap();
        assert!((o.bandwidth - 0.0).abs() < 1e-10);
    }

    #[test]
    fn bb_not_enough_data() {
        let mut bb = BollingerBands::new(5, 2.0);
        for _ in 0..4 {
            assert_eq!(bb.update(1.0), None);
        }
        assert!(bb.update(1.0).is_some());
    }

    #[test]
    fn bb_zero_middle_bandwidth() {
        let mut bb = BollingerBands::new(3, 2.0);
        bb.update(0.0);
        bb.update(0.0);
        assert!((bb.update(0.0).unwrap().bandwidth - 0.0).abs() < 1e-10);
    }

    #[test]
    fn bb_reset() {
        let mut bb = BollingerBands::new(3, 2.0);
        bb.update(1.0);
        bb.update(1.0);
        bb.reset();
        assert_eq!(bb.update(10.0), None);
    }

    #[test]
    #[should_panic]
    fn bb_zero_period_panics() {
        BollingerBands::new(0, 2.0);
    }

    #[test]
    fn keltner_basic() {
        let mut kc = KeltnerChannel::new(3, 1.5);
        assert_eq!(kc.update(10.0, 9.0, 9.5), None);
        assert_eq!(kc.update(10.5, 9.5, 10.0), None);
        assert_eq!(kc.update(11.0, 10.0, 10.5), None);
        let o = kc.update(11.5, 10.5, 11.0);
        assert!(o.is_some());
        let o = o.unwrap();
        assert!(o.upper > o.middle);
        assert!(o.lower < o.middle);
    }

    #[test]
    fn keltner_reset() {
        let mut kc = KeltnerChannel::new(3, 1.5);
        for _ in 0..5 {
            kc.update(1.0, 1.0, 1.0);
        }
        kc.reset();
        assert_eq!(kc.update(1.0, 1.0, 1.0), None);
    }

    #[test]
    #[should_panic]
    fn keltner_zero_period_panics() {
        KeltnerChannel::new(0, 1.5);
    }
}
