// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Local validation for Binance `exchangeInfo` symbol filters.
//!
//! This module deliberately consumes the exchange response instead of carrying
//! a hand-maintained precision table.  It currently understands the common
//! `PRICE_FILTER`, `LOT_SIZE`, and `MIN_NOTIONAL`/`NOTIONAL` fields. Binance
//! represents numeric fields as decimal strings; JSON numbers are accepted as
//! a convenience for fixtures, but production callers should preserve the
//! response unchanged.
//!
//! A zero min/max bound is treated as disabled, matching Binance's documented
//! filter convention.  Price and quantity normalization floors toward zero to
//! the permitted increment: normalization never increases an order's price or
//! exposure. Callers must submit the returned values, not the original input.

use crate::r#trait::{ExchangeError, Result};
use serde_json::Value;
use std::collections::HashMap;

const GRID_EPSILON: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedLimitOrder {
    pub symbol: String,
    pub price: f64,
    pub qty: f64,
}

#[derive(Debug, Clone)]
pub struct SymbolFilters {
    symbol: String,
    tick_size: f64,
    tick_precision: usize,
    min_price: Option<f64>,
    max_price: Option<f64>,
    step_size: f64,
    qty_precision: usize,
    min_qty: Option<f64>,
    max_qty: Option<f64>,
    min_notional: Option<f64>,
}

impl SymbolFilters {
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn tick_size(&self) -> f64 {
        self.tick_size
    }

    pub fn step_size(&self) -> f64 {
        self.step_size
    }

    pub fn min_notional(&self) -> Option<f64> {
        self.min_notional
    }

    /// Floors a limit price to the symbol's tick size and validates its bounds.
    pub fn normalize_limit_price(&self, price: f64) -> Result<f64> {
        self.normalize_value(price, self.tick_size, self.tick_precision, "price")
            .and_then(|price| {
                self.validate_bounds(price, self.min_price, self.max_price, "price")?;
                Ok(price)
            })
    }

    /// Floors a quantity to the symbol's step size and validates its bounds.
    pub fn normalize_quantity(&self, qty: f64) -> Result<f64> {
        self.normalize_value(qty, self.step_size, self.qty_precision, "quantity")
            .and_then(|qty| {
                self.validate_bounds(qty, self.min_qty, self.max_qty, "quantity")?;
                Ok(qty)
            })
    }

    /// Normalizes and validates a limit order, including its minimum notional.
    pub fn normalize_limit_order(&self, price: f64, qty: f64) -> Result<NormalizedLimitOrder> {
        let price = self.normalize_limit_price(price)?;
        let qty = self.normalize_quantity(qty)?;
        let notional = price * qty;
        if !notional.is_finite() {
            return Err(order_error("price * quantity is not finite"));
        }
        if let Some(min_notional) = self.min_notional {
            if notional + grid_tolerance(min_notional) < min_notional {
                return Err(order_error(&format!(
                    "notional {notional} is below minimum {min_notional} for {}",
                    self.symbol
                )));
            }
        }
        Ok(NormalizedLimitOrder {
            symbol: self.symbol.clone(),
            price,
            qty,
        })
    }

    fn normalize_value(
        &self,
        value: f64,
        increment: f64,
        precision: usize,
        field: &str,
    ) -> Result<f64> {
        if !value.is_finite() || value <= 0.0 {
            return Err(order_error(&format!(
                "{field} must be a positive finite number"
            )));
        }
        let steps = (value / increment + GRID_EPSILON).floor();
        let normalized = round_decimal(steps * increment, precision);
        if !normalized.is_finite() || normalized <= 0.0 {
            return Err(order_error(&format!(
                "{field} {value} rounds below the minimum increment {increment}"
            )));
        }
        Ok(normalized)
    }

    fn validate_bounds(
        &self,
        value: f64,
        min: Option<f64>,
        max: Option<f64>,
        field: &str,
    ) -> Result<()> {
        if let Some(min) = min {
            if value + grid_tolerance(min) < min {
                return Err(order_error(&format!(
                    "{field} {value} is below minimum {min}"
                )));
            }
        }
        if let Some(max) = max {
            if value - grid_tolerance(max) > max {
                return Err(order_error(&format!(
                    "{field} {value} exceeds maximum {max}"
                )));
            }
        }
        Ok(())
    }
}

/// Indexes symbol filters parsed from one Binance `exchangeInfo` response.
#[derive(Debug, Clone, Default)]
pub struct BinanceFilters {
    symbols: HashMap<String, SymbolFilters>,
}

impl BinanceFilters {
    pub fn from_exchange_info(value: &Value) -> Result<Self> {
        let symbols = value["symbols"]
            .as_array()
            .ok_or_else(|| rest_error("exchangeInfo response is missing symbols"))?;
        let mut parsed = HashMap::with_capacity(symbols.len());
        for entry in symbols {
            let filters = SymbolFilters::from_exchange_info_symbol(entry)?;
            if parsed.insert(filters.symbol.clone(), filters).is_some() {
                return Err(rest_error("exchangeInfo contains a duplicate symbol"));
            }
        }
        Ok(Self { symbols: parsed })
    }

    pub fn symbol(&self, symbol: &str) -> Result<&SymbolFilters> {
        let symbol = normalize_symbol(symbol)?;
        self.symbols
            .get(&symbol)
            .ok_or_else(|| order_error(&format!("no exchange filters loaded for {symbol}")))
    }
}

impl SymbolFilters {
    fn from_exchange_info_symbol(value: &Value) -> Result<Self> {
        let symbol = normalize_symbol(required_text(value, "symbol")?)?;
        let filters = value["filters"]
            .as_array()
            .ok_or_else(|| rest_error(&format!("exchangeInfo {symbol} is missing filters")))?;
        let price = find_filter(filters, "PRICE_FILTER")
            .ok_or_else(|| rest_error(&format!("exchangeInfo {symbol} is missing PRICE_FILTER")))?;
        let lot = find_filter(filters, "LOT_SIZE")
            .ok_or_else(|| rest_error(&format!("exchangeInfo {symbol} is missing LOT_SIZE")))?;

        let (tick_size, tick_precision) = positive_decimal(price, "tickSize", &symbol)?;
        let (step_size, qty_precision) = positive_decimal(lot, "stepSize", &symbol)?;
        let min_notional = if let Some(filter) = find_filter(filters, "MIN_NOTIONAL") {
            optional_positive_decimal(filter, "minNotional", &symbol)?
                .or(optional_positive_decimal(filter, "notional", &symbol)?)
        } else if let Some(filter) = find_filter(filters, "NOTIONAL") {
            optional_positive_decimal(filter, "minNotional", &symbol)?
        } else {
            None
        };

        Ok(Self {
            symbol,
            tick_size,
            tick_precision,
            min_price: optional_positive_decimal(price, "minPrice", "price")?,
            max_price: optional_positive_decimal(price, "maxPrice", "price")?,
            step_size,
            qty_precision,
            min_qty: optional_positive_decimal(lot, "minQty", "quantity")?,
            max_qty: optional_positive_decimal(lot, "maxQty", "quantity")?,
            min_notional,
        })
    }
}

fn find_filter<'a>(filters: &'a [Value], filter_type: &str) -> Option<&'a Value> {
    filters
        .iter()
        .find(|filter| filter["filterType"].as_str() == Some(filter_type))
}

fn required_text<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value[field]
        .as_str()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| rest_error(&format!("exchangeInfo is missing {field}")))
}

fn positive_decimal(value: &Value, field: &str, symbol: &str) -> Result<(f64, usize)> {
    let text = decimal_value(value, field, symbol)?;
    let parsed = parse_decimal(&text, field, symbol)?;
    if parsed <= 0.0 {
        return Err(rest_error(&format!(
            "exchangeInfo {symbol} has non-positive {field}"
        )));
    }
    Ok((parsed, decimal_precision(&text)))
}

fn optional_positive_decimal(value: &Value, field: &str, symbol: &str) -> Result<Option<f64>> {
    let Some(raw) = value.get(field) else {
        return Ok(None);
    };
    let text = raw_decimal_text(raw, field, symbol)?;
    let parsed = parse_decimal(&text, field, symbol)?;
    Ok((parsed > 0.0).then_some(parsed))
}

fn decimal_value(value: &Value, field: &str, symbol: &str) -> Result<String> {
    let raw = value
        .get(field)
        .ok_or_else(|| rest_error(&format!("exchangeInfo {symbol} is missing {field}")))?;
    raw_decimal_text(raw, field, symbol)
}

fn raw_decimal_text(value: &Value, field: &str, symbol: &str) -> Result<String> {
    value
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
        .or_else(|| value.as_f64().map(|number| number.to_string()))
        .ok_or_else(|| rest_error(&format!("exchangeInfo {symbol} has invalid {field}")))
}

fn parse_decimal(text: &str, field: &str, symbol: &str) -> Result<f64> {
    let parsed = text
        .parse::<f64>()
        .map_err(|_| rest_error(&format!("exchangeInfo {symbol} has invalid {field}")))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(rest_error(&format!(
            "exchangeInfo {symbol} has non-finite {field}"
        )))
    }
}

fn decimal_precision(text: &str) -> usize {
    let lower = text.to_ascii_lowercase();
    let (mantissa, exponent) = lower.split_once('e').unwrap_or((&lower, "0"));
    let fractional = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let exponent = exponent.parse::<i32>().unwrap_or(0);
    (fractional as i32 - exponent).max(0) as usize
}

fn round_decimal(value: f64, precision: usize) -> f64 {
    let factor = 10_f64.powi(precision.min(15) as i32);
    (value * factor).round() / factor
}

fn grid_tolerance(value: f64) -> f64 {
    value.abs().max(1.0) * 1e-12
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let normalized = symbol.trim().to_ascii_uppercase();
    if !(3..=20).contains(&normalized.len())
        || !normalized.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(order_error(
            "symbol must contain 3-20 ASCII alphanumeric characters",
        ));
    }
    Ok(normalized)
}

fn rest_error(message: &str) -> ExchangeError {
    ExchangeError::Rest(message.into())
}

fn order_error(message: &str) -> ExchangeError {
    ExchangeError::Order(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info() -> Value {
        json!({"symbols": [{"symbol": "BTCUSDT", "filters": [
            {"filterType": "PRICE_FILTER", "minPrice": "0.10", "maxPrice": "1000000", "tickSize": "0.10"},
            {"filterType": "LOT_SIZE", "minQty": "0.001", "maxQty": "100", "stepSize": "0.001"},
            {"filterType": "MIN_NOTIONAL", "notional": "5"}
        ]}]})
    }

    #[test]
    fn parses_and_normalizes_a_limit_order() {
        let filters = BinanceFilters::from_exchange_info(&info()).unwrap();
        let order = filters
            .symbol(" btcusdt ")
            .unwrap()
            .normalize_limit_order(50_000.129, 0.00199)
            .unwrap();
        assert_eq!(
            order,
            NormalizedLimitOrder {
                symbol: "BTCUSDT".into(),
                price: 50_000.1,
                qty: 0.001
            }
        );
    }

    #[test]
    fn enforces_bounds_and_notional_after_normalization() {
        let filter = BinanceFilters::from_exchange_info(&info())
            .unwrap()
            .symbol("BTCUSDT")
            .unwrap()
            .clone();
        assert!(filter.normalize_limit_order(50_000.0, 0.0009).is_err());
        assert!(filter
            .normalize_limit_order(4_999.99, 0.001)
            .unwrap_err()
            .to_string()
            .contains("notional"));
        assert!(filter
            .normalize_limit_order(1_000_000.1, 1.0)
            .unwrap_err()
            .to_string()
            .contains("exceeds maximum"));
    }

    #[test]
    fn supports_spot_style_min_notional_and_disabled_zero_bounds() {
        let mut exchange_info = info();
        let symbol = &mut exchange_info["symbols"][0];
        symbol["filters"][0]["minPrice"] = json!("0");
        symbol["filters"][1]["minQty"] = json!("0");
        symbol["filters"][2] = json!({"filterType": "NOTIONAL", "minNotional": "10"});
        let filter = BinanceFilters::from_exchange_info(&exchange_info)
            .unwrap()
            .symbol("BTCUSDT")
            .unwrap()
            .clone();
        assert_eq!(filter.min_notional(), Some(10.0));
        assert!(filter.normalize_limit_order(9.9, 1.0).is_err());
    }

    #[test]
    fn rejects_missing_or_invalid_exchange_info() {
        assert!(BinanceFilters::from_exchange_info(&json!({})).is_err());
        let mut missing = info();
        missing["symbols"][0]["filters"] = json!([]);
        assert!(BinanceFilters::from_exchange_info(&missing).is_err());
        let mut bad_tick = info();
        bad_tick["symbols"][0]["filters"][0]["tickSize"] = json!("0");
        assert!(BinanceFilters::from_exchange_info(&bad_tick).is_err());
    }

    #[test]
    fn rejects_unknown_symbol_and_invalid_inputs() {
        let filters = BinanceFilters::from_exchange_info(&info()).unwrap();
        assert!(filters.symbol("BTC-USDT").is_err());
        assert!(filters.symbol("ETHUSDT").is_err());
        let filter = filters.symbol("BTCUSDT").unwrap();
        assert!(filter.normalize_limit_price(f64::NAN).is_err());
        assert!(filter.normalize_quantity(-1.0).is_err());
    }

    #[test]
    fn decimal_precision_handles_scientific_notation() {
        assert_eq!(decimal_precision("0.00100"), 5);
        assert_eq!(decimal_precision("1e-8"), 8);
        assert_eq!(decimal_precision("1.25e-3"), 5);
    }
}
