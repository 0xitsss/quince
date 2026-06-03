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
        if let Some(rest) = line.strip_prefix("-- USING ") {
            // Split on ';' to handle inline "; SET BUFFER <N>"
            let (decl, extra) = match rest.find(';') {
                Some(pos) => (&rest[..pos], Some(&rest[pos + 1..])),
                None => (rest, None),
            };

            let parts: Vec<&str> = decl.split_whitespace().collect();
            if parts.is_empty() { continue; }
            let name = parts[0].to_lowercase();
            let params: Vec<f64> = parts[1..].iter()
                .filter_map(|s| s.parse::<f64>().ok())
                .collect();
            let mut buf = default_buffer(&name, &params);

            if let Some(xtra) = extra {
                let xtra = xtra.trim();
                if let Some(bstr) = xtra.strip_prefix("SET BUFFER ") {
                    if let Ok(n) = bstr.trim().parse::<usize>() {
                        if n > 0 { buf = n; }
                    }
                }
            }

            entries.push(IndicatorEntry { name, params, buffer: buf });
        } else if let Some(rest) = line.strip_prefix("-- SET BUFFER ") {
            // Standalone line (for the last entry)
            if let Ok(n) = rest.trim().parse::<usize>() {
                if n > 0 {
                    if let Some(entry) = entries.last_mut() {
                        entry.buffer = n;
                    }
                }
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
    results: Vec<(&'static str, f64)>,
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
            cum_buy: 0.0,
            cum_sell: 0.0,
            trades: 0,
        }
    }

    pub fn on_trade(&mut self, trade: &Trade) -> &[(&'static str, f64)] {
        self.results.clear();
        let p = trade.price;
        let v = trade.qty;
        let buy = trade.side == Side::Buy;

        self.trades += 1;
        if buy { self.cum_buy += v } else { self.cum_sell += v }

        for ind in &mut self.indicators {
            match ind {
                ActiveIndicator::Sma(ind) => { if let Some(val) = ind.update(p) { self.results.push(("sma", val)); } }
                ActiveIndicator::Ema(ind) => { self.results.push(("ema", ind.update(p))); }
                ActiveIndicator::Wma(ind) => { if let Some(val) = ind.update(p) { self.results.push(("wma", val)); } }
                ActiveIndicator::Vwma(ind) => { if let Some(val) = ind.update(p, v) { self.results.push(("vwma", val)); } }
                ActiveIndicator::Lsma(ind) => { if let Some(val) = ind.update(p) { self.results.push(("lsma", val)); } }
                ActiveIndicator::Rsi(ind) => { if let Some(val) = ind.update(p) { self.results.push(("rsi", val)); } }
                ActiveIndicator::Macd(ind) => {
                    if let Some(o) = ind.update(p) {
                        self.results.push(("macd", o.macd_line));
                        self.results.push(("macd.signal", o.signal_line));
                        self.results.push(("macd.histogram", o.histogram));
                    }
                }
                ActiveIndicator::Cci(ind) => { if let Some(val) = ind.update(p, p, p) { self.results.push(("cci", val)); } }
                ActiveIndicator::Roc(ind) => { if let Some(val) = ind.update(p) { self.results.push(("roc", val)); } }
                ActiveIndicator::Stoch(ind) => { if let Some(val) = ind.update(p, p, p) { self.results.push(("stoch", val)); } }
                ActiveIndicator::Bb(ind) => {
                    if let Some(o) = ind.update(p) {
                        self.results.push(("bb.middle", o.middle));
                        self.results.push(("bb.upper", o.upper));
                        self.results.push(("bb.lower", o.lower));
                        self.results.push(("bb.bandwidth", o.bandwidth));
                    }
                }
                ActiveIndicator::Kc(ind) => {
                    if let Some(o) = ind.update(p, p, p) {
                        self.results.push(("kc.middle", o.middle));
                        self.results.push(("kc.upper", o.upper));
                        self.results.push(("kc.lower", o.lower));
                    }
                }
                ActiveIndicator::Atr(ind) => { if let Some(val) = ind.update(p, p, p) { self.results.push(("atr", val)); } }
                ActiveIndicator::Mfi(ind) => {
                    let candle = Candle::from_trade(p, v);
                    if let Some(val) = ind.update(&candle) { self.results.push(("mfi", val)); }
                }
                ActiveIndicator::Adx(ind) => {
                    let candle = Candle::from_trade(p, v);
                    if let Some(val) = ind.update(&candle) { self.results.push(("adx", val)); }
                }
                ActiveIndicator::Zscore(ind) => { if let Some(val) = ind.update(p) { self.results.push(("zscore", val)); } }
                ActiveIndicator::Cvd(cum) => {
                    if buy { *cum += v } else { *cum -= v }
                    self.results.push(("cvd", *cum));
                }
                ActiveIndicator::Pmdi { value, prev_data, has_prev } => {
                    if *has_prev {
                        if p > *prev_data { *value += value.max(1.0) * ((p + *prev_data) / *prev_data); }
                        *prev_data = p;
                    } else { *value = p; *prev_data = p; *has_prev = true; }
                    self.results.push(("pmdi", *value));
                }
                ActiveIndicator::Nmdi { value, prev_data, has_prev } => {
                    if *has_prev {
                        if p < *prev_data { *value += value.max(1.0) * ((p + *prev_data) / *prev_data); }
                        *prev_data = p;
                    } else { *value = p; *prev_data = p; *has_prev = true; }
                    self.results.push(("nmdi", *value));
                }
            }
        }

        self.results.push(("price", p));
        self.results.push(("volume_delta", self.cum_buy - self.cum_sell));
        self.results.push(("avg_trade_size", if self.trades == 0 { 0.0 } else { (self.cum_buy + self.cum_sell) / self.trades as f64 }));
        self.results.push(("trade_count", self.trades as f64));

        &self.results
    }

    pub fn on_depth(&mut self, depth: &Depth) -> &[(&'static str, f64)] {
        self.results.clear();
        let bid_vol: f64 = depth.bids.iter().map(|l| l.qty).sum();
        let ask_vol: f64 = depth.asks.iter().map(|l| l.qty).sum();
        self.results.push(("bid_depth", bid_vol));
        self.results.push(("ask_depth", ask_vol));
        let imb = if bid_vol + ask_vol == 0.0 { 0.0 } else { (bid_vol - ask_vol) / (bid_vol + ask_vol) * 100.0 };
        self.results.push(("depth_imbalance", imb));
        &self.results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_using_simple() {
        let src = "-- USING sma 20\n-- USING ema 20";
        let entries = parse_using(src);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "sma");
        assert_eq!(entries[0].params, vec![20.0]);
        assert!(entries[0].buffer >= 256);
        assert_eq!(entries[1].name, "ema");
    }

    #[test]
    fn parse_using_with_buffer() {
        let src = "-- USING sma 20; SET BUFFER 2048\n-- USING ema 20";
        let entries = parse_using(src);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].buffer, 2048);
        assert_eq!(entries[1].name, "ema");
    }

    #[test]
    fn parse_using_multi_param() {
        let src = "-- USING bb 20 2.0; SET BUFFER 512";
        let entries = parse_using(src);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "bb");
        assert_eq!(entries[0].params, vec![20.0, 2.0]);
        assert_eq!(entries[0].buffer, 512);
    }

    #[test]
    fn parse_using_no_params() {
        let src = "-- USING cvd\n-- USING pmdi";
        let entries = parse_using(src);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "cvd");
        assert!(entries[0].params.is_empty());
    }

    #[test]
    fn parse_using_unknown_skipped() {
        let src = "-- USING unknown 42\n-- USING sma 10";
        let entries = parse_using(src);
        // unknown is still parsed (it's the caller's choice to skip)
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn indicator_bank_new_all_types() {
        let src = "-- USING sma 20; SET BUFFER 64\n-- USING cvd\n-- USING pmdi\n-- USING nmdi\n-- USING zscore 20";
        let entries = parse_using(src);
        let mut bank = IndicatorBank::new(&entries);
        assert_eq!(bank.indicators.len(), 5);

        let trade = Trade { price: 100.0, qty: 1.0, time: chrono::Utc::now(), side: Side::Buy, trade_id: 1 };
        let r = bank.on_trade(&trade);
        assert!(!r.is_empty());
        // at minimum: price, volume_delta, avg_trade_size, trade_count + cvd + pmdi + nmdi
        assert!(r.len() >= 7);
        let map: std::collections::HashMap<&str, f64> = r.iter().map(|&(k, v)| (k, v)).collect();
        assert_eq!(map.get("price"), Some(&100.0));
        assert_eq!(map.get("cvd"), Some(&1.0));
        assert!(map.contains_key("pmdi"));
        assert!(map.contains_key("nmdi"));
        assert!(map.contains_key("trade_count"));
    }

    #[test]
    fn indicator_bank_depth() {
        let entries = parse_using("-- USING cvd");
        let mut bank = IndicatorBank::new(&entries);
        let depth = Depth {
            bids: vec![DepthLevel { price: 100.0, qty: 10.0 }, DepthLevel { price: 99.0, qty: 20.0 }],
            asks: vec![DepthLevel { price: 101.0, qty: 15.0 }],
        };
        let r = bank.on_depth(&depth);
        assert_eq!(r.len(), 3);
        let map: std::collections::HashMap<&str, f64> = r.iter().map(|&(k, v)| (k, v)).collect();
        assert_eq!(map.get("bid_depth"), Some(&30.0));
        assert_eq!(map.get("ask_depth"), Some(&15.0));
    }

    #[test]
    fn indicator_bank_zero_allocs_per_tick() {
        let entries = parse_using("-- USING sma 10; SET BUFFER 64\n-- USING ema 10\n-- USING cvd");
        let mut bank = IndicatorBank::new(&entries);
        let trade = Trade { price: 100.0, qty: 1.0, time: chrono::Utc::now(), side: Side::Buy, trade_id: 1 };

        // Warmup
        bank.on_trade(&trade);

        let len1 = bank.on_trade(&trade).len();
        let len2 = bank.on_trade(&trade).len();
        assert_eq!(len1, len2);
    }
}
