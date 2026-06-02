use quince_core::types::OrderFill;
use std::io::{BufWriter, Write};
use std::fs::{File, OpenOptions};

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
            .map(|f| BufWriter::new(f));
        if writer.is_none() {
            tracing::warn!("failed to open trade log: {}", path);
        }
        Self { writer }
    }

    pub fn log_fill(&mut self, fill: &OrderFill) {
        let Some(ref mut writer) = self.writer else { return };
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
    }
}
