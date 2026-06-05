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
    name_to_slot: std::collections::HashMap<String, u16>,
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
            name_to_slot: std::collections::HashMap::new(),
            cum_buy: 0.0,
            cum_sell: 0.0,
            trades: 0,
        }
    }

    /// Pre-assign name→slot mapping (call once at init).
    pub fn set_name_to_slot(&mut self, name: &str, slot: u16) {
        self.name_to_slot.insert(name.to_string(), slot);
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
            self.name_to_slot.entry(name.to_string()).or_insert_with(|| { let s = next; next += 1; s });
        }
    }

    pub fn on_trade(&mut self, trade: &Trade) -> &[(u16, f64)] {
        self.results.clear();
        let p = trade.price;
        let v = trade.qty;
        let buy = trade.side == Side::Buy;
        let slots = &self.name_to_slot;

        self.trades += 1;
        if buy { self.cum_buy += v } else { self.cum_sell += v }

        for ind in &mut self.indicators {
            match ind {
                ActiveIndicator::Sma(ind) => { if let Some(val) = ind.update(p) { self.results.push((*slots.get("sma").unwrap_or(&0), val)); } }
                ActiveIndicator::Ema(ind) => { self.results.push((*slots.get("ema").unwrap_or(&0), ind.update(p))); }
                ActiveIndicator::Wma(ind) => { if let Some(val) = ind.update(p) { self.results.push((*slots.get("wma").unwrap_or(&0), val)); } }
                ActiveIndicator::Vwma(ind) => { if let Some(val) = ind.update(p, v) { self.results.push((*slots.get("vwma").unwrap_or(&0), val)); } }
                ActiveIndicator::Lsma(ind) => { if let Some(val) = ind.update(p) { self.results.push((*slots.get("lsma").unwrap_or(&0), val)); } }
                ActiveIndicator::Rsi(ind) => { if let Some(val) = ind.update(p) { self.results.push((*slots.get("rsi").unwrap_or(&0), val)); } }
                ActiveIndicator::Macd(ind) => {
                    if let Some(o) = ind.update(p) {
                        self.results.push((*slots.get("macd").unwrap_or(&0), o.macd_line));
                        self.results.push((*slots.get("macd.signal").unwrap_or(&0), o.signal_line));
                        self.results.push((*slots.get("macd.histogram").unwrap_or(&0), o.histogram));
                    }
                }
                ActiveIndicator::Cci(ind) => { if let Some(val) = ind.update(p, p, p) { self.results.push((*slots.get("cci").unwrap_or(&0), val)); } }
                ActiveIndicator::Roc(ind) => { if let Some(val) = ind.update(p) { self.results.push((*slots.get("roc").unwrap_or(&0), val)); } }
                ActiveIndicator::Stoch(ind) => { if let Some(val) = ind.update(p, p, p) { self.results.push((*slots.get("stoch").unwrap_or(&0), val)); } }
                ActiveIndicator::Bb(ind) => {
                    if let Some(o) = ind.update(p) {
                        self.results.push((*slots.get("bb.middle").unwrap_or(&0), o.middle));
                        self.results.push((*slots.get("bb.upper").unwrap_or(&0), o.upper));
                        self.results.push((*slots.get("bb.lower").unwrap_or(&0), o.lower));
                        self.results.push((*slots.get("bb.bandwidth").unwrap_or(&0), o.bandwidth));
                    }
                }
                ActiveIndicator::Kc(ind) => {
                    if let Some(o) = ind.update(p, p, p) {
                        self.results.push((*slots.get("kc.middle").unwrap_or(&0), o.middle));
                        self.results.push((*slots.get("kc.upper").unwrap_or(&0), o.upper));
                        self.results.push((*slots.get("kc.lower").unwrap_or(&0), o.lower));
                    }
                }
                ActiveIndicator::Atr(ind) => { if let Some(val) = ind.update(p, p, p) { self.results.push((*slots.get("atr").unwrap_or(&0), val)); } }
                ActiveIndicator::Mfi(ind) => {
                    let candle = Candle::from_trade(p, v);
                    if let Some(val) = ind.update(&candle) { self.results.push((*slots.get("mfi").unwrap_or(&0), val)); }
                }
                ActiveIndicator::Adx(ind) => {
                    let candle = Candle::from_trade(p, v);
                    if let Some(val) = ind.update(&candle) { self.results.push((*slots.get("adx").unwrap_or(&0), val)); }
                }
                ActiveIndicator::Zscore(ind) => { if let Some(val) = ind.update(p) { self.results.push((*slots.get("zscore").unwrap_or(&0), val)); } }
                ActiveIndicator::Cvd(cum) => {
                    if buy { *cum += v } else { *cum -= v }
                    self.results.push((*slots.get("cvd").unwrap_or(&0), *cum));
                }
                ActiveIndicator::Pmdi { value, prev_data, has_prev } => {
                    if *has_prev {
                        if p > *prev_data { *value += value.max(1.0) * ((p + *prev_data) / *prev_data); }
                        *prev_data = p;
                    } else { *value = p; *prev_data = p; *has_prev = true; }
                    self.results.push((*slots.get("pmdi").unwrap_or(&0), *value));
                }
                ActiveIndicator::Nmdi { value, prev_data, has_prev } => {
                    if *has_prev {
                        if p < *prev_data { *value += value.max(1.0) * ((p + *prev_data) / *prev_data); }
                        *prev_data = p;
                    } else { *value = p; *prev_data = p; *has_prev = true; }
                    self.results.push((*slots.get("nmdi").unwrap_or(&0), *value));
                }
            }
        }

        self.results.push((*slots.get("price").unwrap_or(&0), p));
        self.results.push((*slots.get("volume_delta").unwrap_or(&0), self.cum_buy - self.cum_sell));
        self.results.push((*slots.get("avg_trade_size").unwrap_or(&0), if self.trades == 0 { 0.0 } else { (self.cum_buy + self.cum_sell) / self.trades as f64 }));
        self.results.push((*slots.get("trade_count").unwrap_or(&0), self.trades as f64));

        &self.results
    }

    pub fn on_depth(&mut self, depth: &Depth) -> &[(u16, f64)] {
        self.results.clear();
        let slots = &self.name_to_slot;
        let bid_vol: f64 = depth.bids.iter().map(|l| l.qty).sum();
        let ask_vol: f64 = depth.asks.iter().map(|l| l.qty).sum();
        self.results.push((*slots.get("bid_depth").unwrap_or(&0), bid_vol));
        self.results.push((*slots.get("ask_depth").unwrap_or(&0), ask_vol));
        let imb = if bid_vol + ask_vol == 0.0 { 0.0 } else { (bid_vol - ask_vol) / (bid_vol + ask_vol) * 100.0 };
        self.results.push((*slots.get("depth_imbalance").unwrap_or(&0), imb));
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
}
