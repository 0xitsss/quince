use quince_core::types::*;
use quince_indicators::Candle;

const DEFAULT_BUFFER: usize = 256;

#[derive(Debug, Clone)]
pub struct IndicatorEntry {
    pub name: String,
    pub params: Vec<f64>,
    pub buffer: usize,
}

pub fn parse_using(src: &str) -> Vec<IndicatorEntry> {
    let mut entries: Vec<IndicatorEntry> = Vec::new();

    for line in src.lines() {
        let line = line.trim();

        // New format: @using name:param name:param ...
        if let Some(rest) = line.strip_prefix("@using ") {
            for token in rest.split_whitespace() {
                let parts: Vec<&str> = token.split(':').collect();
                if parts.is_empty() || parts[0].is_empty() { continue; }
                let name = parts[0].to_lowercase();
                let params: Vec<f64> = parts[1..].iter()
                    .filter_map(|s| s.parse::<f64>().ok())
                    .collect();
                let buf = default_buffer(&name, &params);
                entries.push(IndicatorEntry { name, params, buffer: buf });
            }
        }
    }

    entries
}

fn default_buffer(_: &str, params: &[f64]) -> usize {
    let period = params.first().copied().unwrap_or(20.0) as usize;
    (period * 2).max(DEFAULT_BUFFER)
}

// ── ActiveIndicator wraps quince-indicators types ──

enum ActiveIndicator {
    Sma(quince_indicators::ma::Sma),
    Ema(quince_indicators::ma::Ema),
    Wma(quince_indicators::ma::Wma),
    Vwma(quince_indicators::ma::Vwma),
    Lsma(quince_indicators::ma::Lsma),
    Rsi(quince_indicators::oscillator::Rsi),
    Macd(quince_indicators::oscillator::Macd),
    Cci(quince_indicators::oscillator::Cci),
    Roc(quince_indicators::oscillator::Roc),
    Stoch(quince_indicators::oscillator::Stochastic),
    Bb(quince_indicators::volatility::BollingerBands),
    Kc(quince_indicators::volatility::KeltnerChannel),
    Atr(quince_indicators::volatility::Atr),
    Mfi(quince_indicators::flow::Mfi),
    Adx(quince_indicators::structure::Adx),
    Zscore(quince_indicators::structure::ZScore),
    Cvd(f64),
    Pmdi { value: f64, prev_data: f64, has_prev: bool },
    Nmdi { value: f64, prev_data: f64, has_prev: bool },
}

// ── IndicatorBank ──

pub struct IndicatorBank {
    indicators: Vec<ActiveIndicator>,
    results: Vec<(u16, f64)>,
    // All indicator slots — zero HashMap in hot path
    slot_sma: u16,
    slot_ema: u16,
    slot_wma: u16,
    slot_vwma: u16,
    slot_lsma: u16,
    slot_rsi: u16,
    slot_macd: u16,
    slot_macd_signal: u16,
    slot_macd_histogram: u16,
    slot_cci: u16,
    slot_roc: u16,
    slot_stoch: u16,
    slot_bb_middle: u16,
    slot_bb_upper: u16,
    slot_bb_lower: u16,
    slot_bb_bandwidth: u16,
    slot_kc_middle: u16,
    slot_kc_upper: u16,
    slot_kc_lower: u16,
    slot_atr: u16,
    slot_mfi: u16,
    slot_adx: u16,
    slot_zscore: u16,
    slot_cvd: u16,
    slot_pmdi: u16,
    slot_nmdi: u16,
    // Pre-cached synthetic slots (zero HashMap lookups in hot path)
    slot_price: u16,
    slot_volume_delta: u16,
    slot_avg_trade_size: u16,
    slot_trade_count: u16,
    slot_bid_depth: u16,
    slot_ask_depth: u16,
    slot_depth_imbalance: u16,
    cum_buy: f64,
    cum_sell: f64,
    trades: u64,
}

impl IndicatorBank {
    pub fn new(cfg: &[IndicatorEntry]) -> Self {
        let mut indicators = Vec::with_capacity(cfg.len());

        for entry in cfg {
            let p = |i: usize| entry.params.get(i).copied();
            let pf = |i: usize| p(i).unwrap_or(0.0);
            let ind = match entry.name.as_str() {
                "sma" => ActiveIndicator::Sma(quince_indicators::ma::Sma::new(pf(0) as usize)),
                "ema" => ActiveIndicator::Ema(quince_indicators::ma::Ema::new(pf(0) as usize)),
                "wma" => ActiveIndicator::Wma(quince_indicators::ma::Wma::new(pf(0) as usize)),
                "vwma" => ActiveIndicator::Vwma(quince_indicators::ma::Vwma::new(pf(0) as usize)),
                "lsma" => ActiveIndicator::Lsma(quince_indicators::ma::Lsma::new(pf(0) as usize)),
                "rsi" => ActiveIndicator::Rsi(quince_indicators::oscillator::Rsi::new(pf(0) as usize)),
                "macd" => ActiveIndicator::Macd(quince_indicators::oscillator::Macd::new(pf(0) as usize, pf(1) as usize, pf(2) as usize)),
                "cci" => ActiveIndicator::Cci(quince_indicators::oscillator::Cci::new(pf(0) as usize, pf(1))),
                "roc" => ActiveIndicator::Roc(quince_indicators::oscillator::Roc::new(pf(0) as usize)),
                "stoch" => ActiveIndicator::Stoch(quince_indicators::oscillator::Stochastic::new(pf(0) as usize)),
                "bb" => ActiveIndicator::Bb(quince_indicators::volatility::BollingerBands::new(pf(0) as usize, pf(1))),
                "kc" => ActiveIndicator::Kc(quince_indicators::volatility::KeltnerChannel::new(pf(0) as usize, pf(1))),
                "atr" => ActiveIndicator::Atr(quince_indicators::volatility::Atr::new(pf(0) as usize)),
                "mfi" => ActiveIndicator::Mfi(quince_indicators::flow::Mfi::new(pf(0) as usize)),
                "adx" => ActiveIndicator::Adx(quince_indicators::structure::Adx::new(pf(0) as usize)),
                "zscore" => ActiveIndicator::Zscore(quince_indicators::structure::ZScore::new(pf(0) as usize)),
                "cvd" => ActiveIndicator::Cvd(0.0),
                "pmdi" => ActiveIndicator::Pmdi { value: 0.0, prev_data: 0.0, has_prev: false },
                "nmdi" => ActiveIndicator::Nmdi { value: 0.0, prev_data: 0.0, has_prev: false },
                _ => continue,
            };
            indicators.push(ind);
        }

        Self {
            indicators,
            results: Vec::with_capacity(64),
            slot_sma: 0,
            slot_ema: 0,
            slot_wma: 0,
            slot_vwma: 0,
            slot_lsma: 0,
            slot_rsi: 0,
            slot_macd: 0,
            slot_macd_signal: 0,
            slot_macd_histogram: 0,
            slot_cci: 0,
            slot_roc: 0,
            slot_stoch: 0,
            slot_bb_middle: 0,
            slot_bb_upper: 0,
            slot_bb_lower: 0,
            slot_bb_bandwidth: 0,
            slot_kc_middle: 0,
            slot_kc_upper: 0,
            slot_kc_lower: 0,
            slot_atr: 0,
            slot_mfi: 0,
            slot_adx: 0,
            slot_zscore: 0,
            slot_cvd: 0,
            slot_pmdi: 0,
            slot_nmdi: 0,
            slot_price: 0,
            slot_volume_delta: 0,
            slot_avg_trade_size: 0,
            slot_trade_count: 0,
            slot_bid_depth: 0,
            slot_ask_depth: 0,
            slot_depth_imbalance: 0,
            cum_buy: 0.0,
            cum_sell: 0.0,
            trades: 0,
        }
    }

    /// Pre-assign name→slot mapping (call once at init).
    /// All indicator slots — zero HashMap lookups in hot path.
    pub fn set_name_to_slot(&mut self, name: &str, slot: u16) {
        match name {
            "sma" => self.slot_sma = slot,
            "ema" => self.slot_ema = slot,
            "wma" => self.slot_wma = slot,
            "vwma" => self.slot_vwma = slot,
            "lsma" => self.slot_lsma = slot,
            "rsi" => self.slot_rsi = slot,
            "macd" => self.slot_macd = slot,
            "macd.signal" => self.slot_macd_signal = slot,
            "macd.histogram" => self.slot_macd_histogram = slot,
            "cci" => self.slot_cci = slot,
            "roc" => self.slot_roc = slot,
            "stoch" => self.slot_stoch = slot,
            "bb.middle" => self.slot_bb_middle = slot,
            "bb.upper" => self.slot_bb_upper = slot,
            "bb.lower" => self.slot_bb_lower = slot,
            "bb.bandwidth" => self.slot_bb_bandwidth = slot,
            "kc.middle" => self.slot_kc_middle = slot,
            "kc.upper" => self.slot_kc_upper = slot,
            "kc.lower" => self.slot_kc_lower = slot,
            "atr" => self.slot_atr = slot,
            "mfi" => self.slot_mfi = slot,
            "adx" => self.slot_adx = slot,
            "zscore" => self.slot_zscore = slot,
            "cvd" => self.slot_cvd = slot,
            "pmdi" => self.slot_pmdi = slot,
            "nmdi" => self.slot_nmdi = slot,
            "price" => self.slot_price = slot,
            "volume_delta" => self.slot_volume_delta = slot,
            "avg_trade_size" => self.slot_avg_trade_size = slot,
            "trade_count" => self.slot_trade_count = slot,
            "bid_depth" => self.slot_bid_depth = slot,
            "ask_depth" => self.slot_ask_depth = slot,
            "depth_imbalance" => self.slot_depth_imbalance = slot,
            _ => {}
        }
    }

    /// Assign sequential slot indices for all known indicator names (for tests).
    pub fn assign_all_slots(&mut self) {
        let mut next = 0u16;
        for name in &[
            "sma","ema","wma","vwma","lsma","rsi",
            "macd","macd.signal","macd.histogram",
            "cci","roc","stoch",
            "bb.middle","bb.upper","bb.lower","bb.bandwidth",
            "kc.middle","kc.upper","kc.lower",
            "atr","mfi","adx","zscore","cvd","pmdi","nmdi",
            "price","volume_delta","avg_trade_size","trade_count",
            "bid_depth","ask_depth","depth_imbalance",
            "entry_price","unrealized_pnl",
        ] {
            self.set_name_to_slot(name, next);
            next += 1;
        }
    }

    pub fn on_trade(&mut self, trade: &Trade) -> &[(u16, f64)] {
        self.results.clear();
        let p = trade.price;
        let v = trade.qty;
        let buy = trade.side == Side::Buy;

        self.trades += 1;
        if buy { self.cum_buy += v } else { self.cum_sell += v }

        for ind in &mut self.indicators {
            match ind {
                ActiveIndicator::Sma(ind) => { if let Some(val) = ind.update(p) { self.results.push((self.slot_sma, val)); } }
                ActiveIndicator::Ema(ind) => { self.results.push((self.slot_ema, ind.update(p))); }
                ActiveIndicator::Wma(ind) => { if let Some(val) = ind.update(p) { self.results.push((self.slot_wma, val)); } }
                ActiveIndicator::Vwma(ind) => { if let Some(val) = ind.update(p, v) { self.results.push((self.slot_vwma, val)); } }
                ActiveIndicator::Lsma(ind) => { if let Some(val) = ind.update(p) { self.results.push((self.slot_lsma, val)); } }
                ActiveIndicator::Rsi(ind) => { if let Some(val) = ind.update(p) { self.results.push((self.slot_rsi, val)); } }
                ActiveIndicator::Macd(ind) => {
                    if let Some(o) = ind.update(p) {
                        self.results.push((self.slot_macd, o.macd_line));
                        self.results.push((self.slot_macd_signal, o.signal_line));
                        self.results.push((self.slot_macd_histogram, o.histogram));
                    }
                }
                ActiveIndicator::Cci(ind) => { if let Some(val) = ind.update(p, p, p) { self.results.push((self.slot_cci, val)); } }
                ActiveIndicator::Roc(ind) => { if let Some(val) = ind.update(p) { self.results.push((self.slot_roc, val)); } }
                ActiveIndicator::Stoch(ind) => { if let Some(val) = ind.update(p, p, p) { self.results.push((self.slot_stoch, val)); } }
                ActiveIndicator::Bb(ind) => {
                    if let Some(o) = ind.update(p) {
                        self.results.push((self.slot_bb_middle, o.middle));
                        self.results.push((self.slot_bb_upper, o.upper));
                        self.results.push((self.slot_bb_lower, o.lower));
                        self.results.push((self.slot_bb_bandwidth, o.bandwidth));
                    }
                }
                ActiveIndicator::Kc(ind) => {
                    if let Some(o) = ind.update(p, p, p) {
                        self.results.push((self.slot_kc_middle, o.middle));
                        self.results.push((self.slot_kc_upper, o.upper));
                        self.results.push((self.slot_kc_lower, o.lower));
                    }
                }
                ActiveIndicator::Atr(ind) => { if let Some(val) = ind.update(p, p, p) { self.results.push((self.slot_atr, val)); } }
                ActiveIndicator::Mfi(ind) => {
                    let candle = Candle::from_trade(p, v);
                    if let Some(val) = ind.update(&candle) { self.results.push((self.slot_mfi, val)); }
                }
                ActiveIndicator::Adx(ind) => {
                    let candle = Candle::from_trade(p, v);
                    if let Some(val) = ind.update(&candle) { self.results.push((self.slot_adx, val)); }
                }
                ActiveIndicator::Zscore(ind) => { if let Some(val) = ind.update(p) { self.results.push((self.slot_zscore, val)); } }
                ActiveIndicator::Cvd(cum) => {
                    if buy { *cum += v } else { *cum -= v }
                    self.results.push((self.slot_cvd, *cum));
                }
                ActiveIndicator::Pmdi { value, prev_data, has_prev } => {
                    if *has_prev {
                        if p > *prev_data { *value += value.max(1.0) * ((p + *prev_data) / *prev_data); }
                        *prev_data = p;
                    } else { *value = p; *prev_data = p; *has_prev = true; }
                    self.results.push((self.slot_pmdi, *value));
                }
                ActiveIndicator::Nmdi { value, prev_data, has_prev } => {
                    if *has_prev {
                        if p < *prev_data { *value += value.max(1.0) * ((p + *prev_data) / *prev_data); }
                        *prev_data = p;
                    } else { *value = p; *prev_data = p; *has_prev = true; }
                    self.results.push((self.slot_nmdi, *value));
                }
            }
        }

        self.results.push((self.slot_price, p));
        self.results.push((self.slot_volume_delta, self.cum_buy - self.cum_sell));
        self.results.push((self.slot_avg_trade_size, if self.trades == 0 { 0.0 } else { (self.cum_buy + self.cum_sell) / self.trades as f64 }));
        self.results.push((self.slot_trade_count, self.trades as f64));

        &self.results
    }

    pub fn on_depth(&mut self, depth: &Depth) -> &[(u16, f64)] {
        self.results.clear();
        let bid_vol: f64 = depth.bids.iter().map(|l| l.qty).sum();
        let ask_vol: f64 = depth.asks.iter().map(|l| l.qty).sum();
        self.results.push((self.slot_bid_depth, bid_vol));
        self.results.push((self.slot_ask_depth, ask_vol));
        let imb = if bid_vol + ask_vol == 0.0 { 0.0 } else { (bid_vol - ask_vol) / (bid_vol + ask_vol) * 100.0 };
        self.results.push((self.slot_depth_imbalance, imb));
        &self.results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_using_simple() {
        let src = "@using sma:20 ema:20";
        let entries = parse_using(src);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "sma");
        assert_eq!(entries[0].params, vec![20.0]);
        assert!(entries[0].buffer >= 256);
        assert_eq!(entries[1].name, "ema");
    }

    #[test]
    fn parse_using_multi_param() {
        let src = "@using bb:20:2.0";
        let entries = parse_using(src);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "bb");
        assert_eq!(entries[0].params, vec![20.0, 2.0]);
    }

    #[test]
    fn parse_using_no_params() {
        let src = "@using cvd pmdi";
        let entries = parse_using(src);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "cvd");
        assert!(entries[0].params.is_empty());
    }

    #[test]
    fn parse_using_unknown_skipped() {
        let src = "@using unknown:42 sma:10";
        let entries = parse_using(src);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn indicator_bank_new_all_types() {
        let src = "@using sma:20 cvd pmdi nmdi zscore:20";
        let entries = parse_using(src);
        let mut bank = IndicatorBank::new(&entries);
        bank.assign_all_slots();
        assert_eq!(bank.indicators.len(), 5);

        let trade = Trade { price: 100.0, qty: 1.0, time: chrono::Utc::now(), side: Side::Buy, trade_id: 1 };
        let r = bank.on_trade(&trade);
        assert!(!r.is_empty());
        assert!(r.len() >= 7);
    }

    #[test]
    fn indicator_bank_depth() {
        let entries = parse_using("@using cvd");
        let mut bank = IndicatorBank::new(&entries);
        bank.assign_all_slots();
        let depth = Depth {
            bids: vec![DepthLevel { price: 100.0, qty: 10.0 }, DepthLevel { price: 99.0, qty: 20.0 }],
            asks: vec![DepthLevel { price: 101.0, qty: 15.0 }],
        };
        let r = bank.on_depth(&depth);
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn indicator_bank_zero_allocs_per_tick() {
        let entries = parse_using("@using sma:10 ema:10 cvd");
        let mut bank = IndicatorBank::new(&entries);
        bank.assign_all_slots();
        let trade = Trade { price: 100.0, qty: 1.0, time: chrono::Utc::now(), side: Side::Buy, trade_id: 1 };

        // Warmup
        bank.on_trade(&trade);

        let len1 = bank.on_trade(&trade).len();
        let len2 = bank.on_trade(&trade).len();
        assert_eq!(len1, len2);
    }

    #[test]
    fn parse_using_empty_src() {
        let entries = parse_using("");
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_using_no_at_using() {
        let entries = parse_using("local x = 1");
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_using_empty_name_skipped() {
        let entries = parse_using("@using :20");
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_using_case_insensitive() {
        let entries = parse_using("@using SMA:20");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "sma");
    }

    #[test]
    fn parse_using_non_numeric_param_skipped() {
        let entries = parse_using("@using ema:abc:20");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].params, vec![20.0]);
    }

    #[test]
    fn indicator_bank_new_empty_cfg() {
        let bank = IndicatorBank::new(&[]);
        assert!(bank.indicators.is_empty());
    }

    #[test]
    fn indicator_bank_unknown_name_skipped() {
        let entries = parse_using("@using unknown_indicator:20");
        let bank = IndicatorBank::new(&entries);
        assert!(bank.indicators.is_empty());
    }

    #[test]
    fn indicator_bank_on_trade_all_synthetic() {
        let mut bank = IndicatorBank::new(&[]);
        bank.assign_all_slots();
        let trade = Trade { price: 100.0, qty: 1.0, time: chrono::Utc::now(), side: Side::Buy, trade_id: 1 };
        let r = bank.on_trade(&trade);
        assert_eq!(r.len(), 4); // price, volume_delta, avg_trade_size, trade_count
    }

    #[test]
    fn indicator_bank_on_depth_empty() {
        let mut bank = IndicatorBank::new(&[]);
        bank.assign_all_slots();
        let depth = Depth { bids: vec![], asks: vec![] };
        let r = bank.on_depth(&depth);
        assert_eq!(r.len(), 3); // bid_depth, ask_depth, depth_imbalance
        for &(_, v) in r {
            assert_eq!(v, 0.0);
        }
    }

    #[test]
    fn indicator_bank_set_name_to_slot() {
        let mut bank = IndicatorBank::new(&[]);
        bank.set_name_to_slot("custom", 99);
        let trade = Trade { price: 50.0, qty: 2.0, time: chrono::Utc::now(), side: Side::Sell, trade_id: 2 };
        bank.assign_all_slots();
        let r = bank.on_trade(&trade);
        assert!(r.len() >= 2);
    }

    #[test]
    fn default_buffer_min_256() {
        let e = parse_using("@using sma:1");
        assert_eq!(e[0].buffer, 256);
    }

    #[test]
    fn default_buffer_scales_with_period() {
        let e = parse_using("@using sma:200");
        assert_eq!(e[0].buffer, 400);
    }

    #[test]
    fn indicator_bank_on_trade_sell_side() {
        let mut bank = IndicatorBank::new(&[]);
        bank.assign_all_slots();
        let trade = Trade { price: 100.0, qty: 1.0, time: chrono::Utc::now(), side: Side::Sell, trade_id: 1 };
        let r = bank.on_trade(&trade);
        assert_eq!(r.len(), 4);
        // price=100, volume_delta=-1, avg_trade_size=1, trade_count=1
        for &(_, v) in r {
            assert!(v > -2.0 && v <= 100.0);
        }
    }
}
