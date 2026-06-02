use quince_core::types::*;
use quince_indicators::{
    ma::*, oscillator::*, volatility::*, flow::*, structure::*, Candle,
};
use std::collections::HashMap;

macro_rules! upd_single {
    ($self:ident, $opt:ident, $key:expr, $val:expr, $m:ident) => {
        if let Some(ref mut ind) = $self.$opt {
            if let Some(r) = ind.update($val) {
                $m.insert($key.into(), r);
            }
        }
    };
}

macro_rules! upd_triple {
    ($self:ident, $opt:ident, $key:expr, $a:expr, $b:expr, $c:expr, $m:ident) => {
        if let Some(ref mut ind) = $self.$opt {
            if let Some(r) = ind.update($a, $b, $c) {
                $m.insert($key.into(), r);
            }
        }
    };
}

#[derive(Debug, Clone)]
pub struct IndicatorConfig {
    pub sma: Option<usize>,
    pub ema: Option<usize>,
    pub wma: Option<usize>,
    pub vwma: Option<usize>,
    pub lsma: Option<usize>,
    pub rsi: Option<usize>,
    pub macd: Option<(usize, usize, usize)>,
    pub cci: Option<(usize, f64)>,
    pub roc: Option<usize>,
    pub stoch: Option<usize>,
    pub bb: Option<(usize, f64)>,
    pub kc: Option<(usize, f64)>,
    pub atr: Option<usize>,
    pub mfi: Option<usize>,
    pub adx: Option<usize>,
    pub cvd: bool,
    pub pmdi: bool,
    pub nmdi: bool,
    pub zscore: Option<usize>,
}

/// Parse `USING` declarations from a strategy file.
/// Lines matching `-- USING <name> [params...]` configure indicators.
pub fn parse_using(src: &str) -> IndicatorConfig {
    let mut cfg = IndicatorConfig {
        sma: None, ema: None, wma: None, vwma: None, lsma: None,
        rsi: None, macd: None, cci: None, roc: None, stoch: None,
        bb: None, kc: None, atr: None, mfi: None, adx: None,
        cvd: false, pmdi: false, nmdi: false, zscore: None,
    };

    for line in src.lines() {
        let line = line.trim();
        let rest = match line.strip_prefix("-- USING ") {
            Some(r) => r,
            _ => continue,
        };
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.is_empty() { continue; }
        let name = parts[0].to_lowercase();
        let p = |i: usize| parts.get(i).and_then(|s| s.parse::<usize>().ok());
        let pf = |i: usize| parts.get(i).and_then(|s| s.parse::<f64>().ok());
        match name.as_str() {
            "sma" => cfg.sma = p(1),
            "ema" => cfg.ema = p(1),
            "wma" => cfg.wma = p(1),
            "vwma" => cfg.vwma = p(1),
            "lsma" => cfg.lsma = p(1),
            "rsi" => cfg.rsi = p(1),
            "roc" => cfg.roc = p(1),
            "stoch" => cfg.stoch = p(1),
            "atr" => cfg.atr = p(1),
            "mfi" => cfg.mfi = p(1),
            "adx" => cfg.adx = p(1),
            "zscore" => cfg.zscore = p(1),
            "macd" => {
                if let (Some(f), Some(s), Some(sig)) = (p(1), p(2), p(3)) {
                    cfg.macd = Some((f, s, sig));
                }
            }
            "cci" => {
                if let (Some(per), Some(c)) = (p(1), pf(2)) {
                    cfg.cci = Some((per, c));
                }
            }
            "bb" => {
                if let (Some(per), Some(m)) = (p(1), pf(2)) {
                    cfg.bb = Some((per, m));
                }
            }
            "kc" => {
                if let (Some(per), Some(m)) = (p(1), pf(2)) {
                    cfg.kc = Some((per, m));
                }
            }
            "cvd" => cfg.cvd = true,
            "pmdi" => cfg.pmdi = true,
            "nmdi" => cfg.nmdi = true,
            _ => {}
        }
    }
    cfg
}

pub struct IndicatorBank {
    sma: Option<Sma>,
    ema: Option<Ema>,
    wma: Option<Wma>,
    vwma: Option<Vwma>,
    lsma: Option<Lsma>,
    rsi: Option<Rsi>,
    macd: Option<Macd>,
    cci: Option<Cci>,
    roc: Option<Roc>,
    stoch: Option<Stochastic>,
    bb: Option<BollingerBands>,
    kc: Option<KeltnerChannel>,
    atr: Option<Atr>,
    mfi: Option<Mfi>,
    adx: Option<Adx>,
    cvd: Cvd,
    pmdi: Pmdi,
    nmdi: Nmdi,
    zscore: Option<ZScore>,
    cum_buy: f64,
    cum_sell: f64,
    trades: u64,
}

impl IndicatorBank {
    pub fn new(cfg: &IndicatorConfig) -> Self {
        Self {
            sma: cfg.sma.map(Sma::new),
            ema: cfg.ema.map(Ema::new),
            wma: cfg.wma.map(Wma::new),
            vwma: cfg.vwma.map(Vwma::new),
            lsma: cfg.lsma.map(Lsma::new),
            rsi: cfg.rsi.map(Rsi::new),
            macd: cfg.macd.map(|(f, s, sig)| Macd::new(f, s, sig)),
            cci: cfg.cci.map(|(p, c)| Cci::new(p, c)),
            roc: cfg.roc.map(Roc::new),
            stoch: cfg.stoch.map(Stochastic::new),
            bb: cfg.bb.map(|(p, m)| BollingerBands::new(p, m)),
            kc: cfg.kc.map(|(p, m)| KeltnerChannel::new(p, m)),
            atr: cfg.atr.map(Atr::new),
            mfi: cfg.mfi.map(Mfi::new),
            adx: cfg.adx.map(Adx::new),
            cvd: Cvd::new(),
            pmdi: Pmdi::new(),
            nmdi: Nmdi::new(),
            zscore: cfg.zscore.map(ZScore::new),
            cum_buy: 0.0, cum_sell: 0.0, trades: 0,
        }
    }

    pub fn on_trade(&mut self, trade: &Trade) -> HashMap<String, f64> {
        let p = trade.price;
        let v = trade.qty;
        self.trades += 1;
        if trade.side == Side::Buy { self.cum_buy += v } else { self.cum_sell += v }

        let candle = Candle::from_trade(p, v);
        let mut m = HashMap::new();

        upd_single!(self, sma, "sma", p, m);
        if let Some(ref mut ind) = self.ema { m.insert("ema".into(), ind.update(p)); }
        upd_single!(self, wma, "wma", p, m);
        if let Some(ref mut ind) = self.vwma { if let Some(r) = ind.update(p, v) { m.insert("vwma".into(), r); } }
        upd_single!(self, lsma, "lsma", p, m);
        upd_single!(self, rsi, "rsi", p, m);
        if let Some(ref mut ind) = self.macd {
            if let Some(o) = ind.update(p) {
                m.insert("macd".into(), o.macd_line);
                m.insert("macd.signal".into(), o.signal_line);
                m.insert("macd.histogram".into(), o.histogram);
            }
        }
        upd_triple!(self, cci, "cci", p, p, p, m);
        upd_single!(self, roc, "roc", p, m);
        upd_triple!(self, stoch, "stoch", p, p, p, m);
        if let Some(ref mut ind) = self.bb {
            if let Some(o) = ind.update(p) {
                m.insert("bb.middle".into(), o.middle);
                m.insert("bb.upper".into(), o.upper);
                m.insert("bb.lower".into(), o.lower);
                m.insert("bb.bandwidth".into(), o.bandwidth);
            }
        }
        if let Some(ref mut ind) = self.kc {
            if let Some(o) = ind.update(p, p, p) {
                m.insert("kc.middle".into(), o.middle);
                m.insert("kc.upper".into(), o.upper);
                m.insert("kc.lower".into(), o.lower);
            }
        }
        upd_triple!(self, atr, "atr", p, p, p, m);
        if let Some(ref mut ind) = self.mfi {
            if let Some(r) = ind.update(&candle) { m.insert("mfi".into(), r); }
        }
        if let Some(ref mut ind) = self.adx {
            if let Some(r) = ind.update(&candle) { m.insert("adx".into(), r); }
        }

        m.insert("cvd".into(), self.cvd.update(p, v, trade.side == Side::Buy));
        m.insert("pmdi".into(), self.pmdi.update(p, v));
        m.insert("nmdi".into(), self.nmdi.update(p, v));
        m.insert("volume_delta".into(), VolumeDelta::update(self.cum_buy, self.cum_sell));
        m.insert("avg_trade_size".into(), AverageTradeSize::update(self.cum_buy + self.cum_sell, self.trades as f64));
        upd_single!(self, zscore, "zscore", p, m);
        m.insert("price".into(), p);
        m.insert("trade_count".into(), self.trades as f64);
        m
    }

    pub fn on_depth(&mut self, depth: &Depth) -> HashMap<String, f64> {
        let bid_vol: f64 = depth.bids.iter().map(|l| l.qty).sum();
        let ask_vol: f64 = depth.asks.iter().map(|l| l.qty).sum();
        let mut m = HashMap::new();
        m.insert("bid_depth".into(), bid_vol);
        m.insert("ask_depth".into(), ask_vol);
        m.insert("depth_imbalance".into(), BidAskImbalance::update(bid_vol, ask_vol));
        m
    }
}
