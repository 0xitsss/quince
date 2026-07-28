//! Streaming importer for reconstructed OKX/Tardis `book_snapshot_25` CSV.
//!
//! Input is read from stdin so compressed archives can be decompressed outside
//! the process without buffering a trading day in memory.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufWriter, Write};

pub fn import_snapshot_25(symbol: &str, output: &str) -> Result<u64, String> {
    if symbol.trim().is_empty() {
        return Err("symbol must not be empty".into());
    }
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let header = lines
        .next()
        .ok_or_else(|| "missing CSV header".to_string())?
        .map_err(|e| e.to_string())?;
    let columns: HashMap<&str, usize> = header
        .split(',')
        .enumerate()
        .map(|(i, name)| (name, i))
        .collect();
    let required = [
        "symbol",
        "timestamp",
        "bids[0].price",
        "bids[0].amount",
        "bids[1].price",
        "bids[1].amount",
        "bids[2].price",
        "bids[2].amount",
        "asks[0].price",
        "asks[0].amount",
        "asks[1].price",
        "asks[1].amount",
        "asks[2].price",
        "asks[2].amount",
    ];
    if required.iter().any(|name| !columns.contains_key(name)) {
        return Err("CSV is not a book_snapshot_25 dataset with top-3 levels".into());
    }
    let mut writer =
        BufWriter::new(File::create(output).map_err(|e| format!("create {output}: {e}"))?);
    let mut imported = 0_u64;
    for (line_no, line) in lines.enumerate() {
        let line = line.map_err(|e| format!("read CSV line {}: {e}", line_no + 2))?;
        let values: Vec<&str> = line.split(',').collect();
        let get = |name: &str| -> Result<&str, String> {
            values
                .get(columns[name])
                .copied()
                .ok_or_else(|| format!("CSV line {} missing {name}", line_no + 2))
        };
        if get("symbol")? != symbol {
            continue;
        }
        let timestamp_us: i64 = get("timestamp")?
            .parse()
            .map_err(|_| format!("CSV line {} invalid timestamp", line_no + 2))?;
        let timestamp_ms = timestamp_us
            .checked_div(1_000)
            .ok_or_else(|| format!("CSV line {} invalid timestamp", line_no + 2))?;
        let side = |name: &str| -> Result<serde_json::Value, String> {
            let levels = (0..3)
                .map(|level| {
                    let price: f64 = get(&format!("{name}[{level}].price"))?
                        .parse()
                        .map_err(|_| format!("CSV line {} invalid {name} price", line_no + 2))?;
                    let qty: f64 = get(&format!("{name}[{level}].amount"))?
                        .parse()
                        .map_err(|_| format!("CSV line {} invalid {name} amount", line_no + 2))?;
                    if !price.is_finite() || price <= 0.0 || !qty.is_finite() || qty <= 0.0 {
                        return Err(format!(
                            "CSV line {} has non-positive {name} level",
                            line_no + 2
                        ));
                    }
                    Ok(serde_json::json!({"price":price,"qty":qty}))
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(serde_json::Value::Array(levels))
        };
        let event = serde_json::json!({"schema_version":1,"type":"depth","bids":side("bids")?,"asks":side("asks")?,"timestamp_ms":timestamp_ms});
        serde_json::to_writer(&mut writer, &event).map_err(|e| e.to_string())?;
        writer.write_all(b"\n").map_err(|e| e.to_string())?;
        imported += 1;
    }
    writer.flush().map_err(|e| e.to_string())?;
    if imported == 0 {
        return Err(format!("no rows found for {symbol}"));
    }
    Ok(imported)
}

pub fn import_trades(symbol: &str, output: &str) -> Result<u64, String> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let header = lines
        .next()
        .ok_or_else(|| "missing CSV header".to_string())?
        .map_err(|e| e.to_string())?;
    let columns: HashMap<&str, usize> = header
        .split(',')
        .enumerate()
        .map(|(i, name)| (name, i))
        .collect();
    for field in ["symbol", "timestamp", "id", "side", "price", "amount"] {
        if !columns.contains_key(field) {
            return Err(format!("CSV is missing {field}"));
        }
    }
    let mut writer =
        BufWriter::new(File::create(output).map_err(|e| format!("create {output}: {e}"))?);
    let mut imported = 0_u64;
    for (line_no, line) in lines.enumerate() {
        let values: Vec<String> = line
            .map_err(|e| e.to_string())?
            .split(',')
            .map(str::to_owned)
            .collect();
        let get = |name: &str| -> Result<&str, String> {
            values
                .get(columns[name])
                .map(String::as_str)
                .ok_or_else(|| format!("CSV line {} missing {name}", line_no + 2))
        };
        if get("symbol")? != symbol {
            continue;
        }
        let timestamp_us: i64 = get("timestamp")?
            .parse()
            .map_err(|_| format!("CSV line {} invalid timestamp", line_no + 2))?;
        let trade_id: u64 = get("id")?
            .parse()
            .map_err(|_| format!("CSV line {} invalid id", line_no + 2))?;
        let price: f64 = get("price")?
            .parse()
            .map_err(|_| format!("CSV line {} invalid price", line_no + 2))?;
        let qty: f64 = get("amount")?
            .parse()
            .map_err(|_| format!("CSV line {} invalid amount", line_no + 2))?;
        if !price.is_finite() || price <= 0.0 || !qty.is_finite() || qty <= 0.0 {
            return Err(format!("CSV line {} has non-positive trade", line_no + 2));
        }
        let side = match get("side")? {
            "buy" => "buy",
            "sell" => "sell",
            _ => return Err(format!("CSV line {} invalid side", line_no + 2)),
        };
        serde_json::to_writer(&mut writer, &serde_json::json!({"schema_version":1,"type":"trade","timestamp_ms":timestamp_us / 1_000,"price":price,"qty":qty,"side":side,"trade_id":trade_id})).map_err(|e| e.to_string())?;
        writer.write_all(b"\n").map_err(|e| e.to_string())?;
        imported += 1;
    }
    writer.flush().map_err(|e| e.to_string())?;
    if imported == 0 {
        return Err(format!("no rows found for {symbol}"));
    }
    Ok(imported)
}
