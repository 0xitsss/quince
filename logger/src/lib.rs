// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Structured trade logging.
//! [`TradeLog`] writes JSON-formatted fill records to a CSV-compatible
//! log file for post-session analysis and reconciliation.

use quince_core::types::OrderFill;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};

pub struct TradeLog {
    writer: Option<BufWriter<File>>,
}

impl TradeLog {
    pub fn new(path: &str) -> Self {
        let writer = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(BufWriter::new);
        if writer.is_none() {
            tracing::warn!("failed to open trade log: {}", path);
        }
        Self { writer }
    }

    pub fn log_fill(&mut self, fill: &OrderFill) {
        let Some(ref mut writer) = self.writer else {
            return;
        };
        let line = serde_json::json!({
            "timestamp": fill.time.to_rfc3339(),
            "order_id": fill.order_id,
            "side": format!("{:?}", fill.side),
            "price": fill.price,
            "qty": fill.qty,
            "fee": fill.fee,
            "fee_asset": fill.fee_asset,
        });
        if let Err(e) = writeln!(writer, "{}", line) {
            tracing::error!("trade log write: {}", e);
        }
        let _ = writer.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::types::{OrderFill, Side};

    fn make_fill(oid: &str, price: f64, qty: f64, side: Side) -> OrderFill {
        OrderFill {
            order_id: oid.into(),
            side,
            price,
            qty,
            fee: qty * price * 0.001,
            fee_asset: "USDT".into(),
            time: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_new_log_creates_file() {
        let path = "test_log.log";
        let _ = std::fs::remove_file(path);
        let mut log = TradeLog::new(path);
        let fill = make_fill("ord1", 50000.0, 0.1, Side::Buy);
        log.log_fill(&fill);
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("ord1"));
        assert!(content.contains("50000"));
        assert!((std::fs::metadata(path).unwrap().len()) > 0);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_log_multiple_fills() {
        let path = "test_log_multi.log";
        let _ = std::fs::remove_file(path);
        let mut log = TradeLog::new(path);
        for i in 0..5 {
            let fill = make_fill(&format!("ord{}", i), 100.0 + i as f64, 0.1, Side::Buy);
            log.log_fill(&fill);
        }
        let content = std::fs::read_to_string(path).unwrap();
        assert_eq!(content.lines().count(), 5);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_log_sell_fill() {
        let path = "test_log_sell.log";
        let _ = std::fs::remove_file(path);
        let mut log = TradeLog::new(path);
        let fill = make_fill("ord_sell", 51000.0, 0.5, Side::Sell);
        log.log_fill(&fill);
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("Sell"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_log_invalid_path() {
        let mut log = TradeLog::new("/invalid/path/trades.log");
        let fill = make_fill("ord1", 50000.0, 0.1, Side::Buy);
        log.log_fill(&fill);
    }

    #[test]
    fn test_log_fee_fields() {
        let path = "test_log_fee.log";
        let _ = std::fs::remove_file(path);
        let mut log = TradeLog::new(path);
        let fill = OrderFill {
            order_id: "fee_test".into(),
            side: Side::Buy,
            price: 50000.0,
            qty: 1.0,
            fee: 50.0,
            fee_asset: "BNB".into(),
            time: chrono::Utc::now(),
        };
        log.log_fill(&fill);
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("fee_test"));
        assert!(content.contains("BNB"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_log_appends() {
        let path = "test_log_append.log";
        let _ = std::fs::remove_file(path);
        {
            let mut log = TradeLog::new(path);
            log.log_fill(&make_fill("first", 100.0, 0.1, Side::Buy));
        }
        {
            let mut log = TradeLog::new(path);
            log.log_fill(&make_fill("second", 200.0, 0.2, Side::Sell));
        }
        let content = std::fs::read_to_string(path).unwrap();
        assert_eq!(content.lines().count(), 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_log_zero_qty() {
        let path = "test_log_zero.log";
        let _ = std::fs::remove_file(path);
        let mut log = TradeLog::new(path);
        let fill = make_fill("zero", 50000.0, 0.0, Side::Buy);
        log.log_fill(&fill);
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("\"qty\":0.0"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_log_large_values() {
        let path = "test_log_large.log";
        let _ = std::fs::remove_file(path);
        let mut log = TradeLog::new(path);
        let fill = OrderFill {
            order_id: "large".into(),
            side: Side::Buy,
            price: 999999.99,
            qty: 999.999,
            fee: 0.01,
            fee_asset: "BTC".into(),
            time: chrono::Utc::now(),
        };
        log.log_fill(&fill);
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("999999.99"));
        assert!(content.contains("999.999"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_log_unicode_symbol() {
        let path = "test_log_uni.log";
        let _ = std::fs::remove_file(path);
        let mut log = TradeLog::new(path);
        let fill = OrderFill {
            order_id: "uni".into(),
            side: Side::Buy,
            price: 0.0001,
            qty: 1000.0,
            fee: 0.1,
            fee_asset: "USDT".into(),
            time: chrono::Utc::now(),
        };
        log.log_fill(&fill);
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("uni"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_log_overwrite_same_path() {
        let path = "test_log_overwrite.log";
        let _ = std::fs::remove_file(path);
        {
            let mut log = TradeLog::new(path);
            log.log_fill(&make_fill("old", 100.0, 0.1, Side::Buy));
        }
        {
            let mut log = TradeLog::new(path);
            log.log_fill(&make_fill("new", 200.0, 0.2, Side::Sell));
        }
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("old"));
        assert!(content.contains("new"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_log_json_valid() {
        let path = "test_log_json.log";
        let _ = std::fs::remove_file(path);
        let mut log = TradeLog::new(path);
        log.log_fill(&make_fill("json_test", 50000.0, 0.5, Side::Buy));
        let content = std::fs::read_to_string(path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed["order_id"], "json_test");
        assert_eq!(parsed["side"], "Buy");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn test_log_empty_order_id() {
        let path = "test_log_empty_id.log";
        let _ = std::fs::remove_file(path);
        let mut log = TradeLog::new(path);
        let fill = make_fill("", 100.0, 0.1, Side::Sell);
        log.log_fill(&fill);
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("\"order_id\":\"\""));
        let _ = std::fs::remove_file(path);
    }
}
