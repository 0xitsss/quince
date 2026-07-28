//! Deterministic, offline merger for independently captured replay streams.
//!
//! This tool is deliberately narrow: it joins one converted trade capture and
//! one converted depth capture.  It opens no network connection and preserves
//! every input JSON object verbatim.  When the millisecond timestamps tie, a
//! trade is emitted before depth.  That conservative ordering prevents a
//! same-timestamp depth snapshot from influencing a preceding trade; ties are
//! counted in the report because their true exchange ordering is unknown.

use serde::Serialize;
use serde_json::Value;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};

const SCHEMA_VERSION: u64 = 1;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MergeSummary {
    pub trades: u64,
    pub depth_snapshots: u64,
    pub timestamp_ties: u64,
    pub output: String,
}

#[derive(Debug)]
struct CaptureRecord {
    timestamp_ms: i64,
    line: String,
}

struct CaptureReader {
    lines: std::io::Lines<BufReader<File>>,
    line_no: usize,
    previous_timestamp_ms: Option<i64>,
    expected_type: &'static str,
    source: &'static str,
}

impl CaptureReader {
    fn open(path: &str, source: &'static str, expected_type: &'static str) -> Result<Self, String> {
        let file =
            File::open(path).map_err(|error| format!("open {source} capture {path}: {error}"))?;
        Ok(Self {
            lines: BufReader::new(file).lines(),
            line_no: 0,
            previous_timestamp_ms: None,
            expected_type,
            source,
        })
    }

    fn next(&mut self) -> Result<Option<CaptureRecord>, String> {
        for next_line in self.lines.by_ref() {
            self.line_no += 1;
            let line = next_line.map_err(|error| {
                format!(
                    "read {} capture line {}: {error}",
                    self.source, self.line_no
                )
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let timestamp_ms = self.validate(&line)?;
            if let Some(previous) = self.previous_timestamp_ms {
                if timestamp_ms < previous {
                    return Err(format!(
                        "{} capture line {} timestamp_ms moved backwards: {timestamp_ms} is before {previous}",
                        self.source, self.line_no
                    ));
                }
            }
            self.previous_timestamp_ms = Some(timestamp_ms);
            return Ok(Some(CaptureRecord { timestamp_ms, line }));
        }
        Ok(None)
    }

    fn validate(&self, line: &str) -> Result<i64, String> {
        let event: Value = serde_json::from_str(line).map_err(|error| {
            format!(
                "{} capture line {} is not JSON: {error}",
                self.source, self.line_no
            )
        })?;
        let object = event.as_object().ok_or_else(|| {
            format!(
                "{} capture line {} must be a JSON object",
                self.source, self.line_no
            )
        })?;
        let schema_version = object
            .get("schema_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                format!(
                    "{} capture line {} missing integer schema_version",
                    self.source, self.line_no
                )
            })?;
        if schema_version != SCHEMA_VERSION {
            return Err(format!(
                "{} capture line {} has unsupported schema_version {schema_version}; expected {SCHEMA_VERSION}",
                self.source, self.line_no
            ));
        }
        let event_type = object.get("type").and_then(Value::as_str).ok_or_else(|| {
            format!(
                "{} capture line {} missing string type",
                self.source, self.line_no
            )
        })?;
        if event_type != self.expected_type {
            return Err(format!(
                "{} capture line {} has type {event_type:?}; expected {:?}",
                self.source, self.line_no, self.expected_type
            ));
        }
        object
            .get("timestamp_ms")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                format!(
                    "{} capture line {} missing integer timestamp_ms",
                    self.source, self.line_no
                )
            })
    }
}

/// Merge converted trade and depth JSONL captures in market-time order.
///
/// Input files must each be nondecreasing by `timestamp_ms`.  Same-millisecond
/// cross-stream events are allowed but reported; trades come first as the
/// conservative deterministic tie-breaker described in this module's docs.
pub fn merge(
    trades_path: &str,
    depth_path: &str,
    output_path: &str,
) -> Result<MergeSummary, String> {
    // Validate complete source files before creating output.  A rejected input
    // must never leave behind a plausible-looking partial merged capture.
    validate_capture(trades_path, "trade", "trade")?;
    validate_capture(depth_path, "depth", "depth")?;
    let mut trades = CaptureReader::open(trades_path, "trade", "trade")?;
    let mut depth = CaptureReader::open(depth_path, "depth", "depth")?;
    let output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output_path)
        .map_err(|error| format!("create new merged capture {output_path}: {error}"))?;
    let mut output = BufWriter::new(output);
    let mut next_trade = trades.next()?;
    let mut next_depth = depth.next()?;
    let mut summary = MergeSummary {
        trades: 0,
        depth_snapshots: 0,
        timestamp_ties: 0,
        output: output_path.into(),
    };

    while next_trade.is_some() || next_depth.is_some() {
        let take_trade = match (&next_trade, &next_depth) {
            (Some(trade), Some(depth)) => {
                if trade.timestamp_ms == depth.timestamp_ms {
                    summary.timestamp_ties += 1;
                }
                // On a tie trade wins: see module-level safety rationale.
                trade.timestamp_ms <= depth.timestamp_ms
            }
            (Some(_), None) => true,
            (None, Some(_)) => false,
            (None, None) => unreachable!("loop condition guarantees an event"),
        };
        let record = if take_trade {
            summary.trades += 1;
            next_trade.take().expect("checked above")
        } else {
            summary.depth_snapshots += 1;
            next_depth.take().expect("checked above")
        };
        output
            .write_all(record.line.as_bytes())
            .and_then(|_| output.write_all(b"\n"))
            .map_err(|error| format!("write merged capture {output_path}: {error}"))?;
        if take_trade {
            next_trade = trades.next()?;
        } else {
            next_depth = depth.next()?;
        }
    }
    output
        .flush()
        .map_err(|error| format!("flush merged capture {output_path}: {error}"))?;
    Ok(summary)
}

fn validate_capture(
    path: &str,
    source: &'static str,
    expected_type: &'static str,
) -> Result<(), String> {
    let mut reader = CaptureReader::open(path, source, expected_type)?;
    while reader.next()?.is_some() {}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::merge;
    use std::fs;

    fn temp(name: &str) -> String {
        std::env::temp_dir()
            .join(format!(
                "quince-capture-merge-{}-{name}",
                std::process::id()
            ))
            .display()
            .to_string()
    }

    #[test]
    fn merges_in_time_order_and_reports_conservative_ties() {
        let trades = temp("trades.jsonl");
        let depth = temp("depth.jsonl");
        let output = temp("merged.jsonl");
        let _ = fs::remove_file(&output);
        fs::write(&trades, "{\"schema_version\":1,\"type\":\"trade\",\"timestamp_ms\":10}\n{\"schema_version\":1,\"type\":\"trade\",\"timestamp_ms\":30}\n").unwrap();
        fs::write(&depth, "{\"schema_version\":1,\"type\":\"depth\",\"timestamp_ms\":20}\n{\"schema_version\":1,\"type\":\"depth\",\"timestamp_ms\":30}\n").unwrap();

        let summary = merge(&trades, &depth, &output).unwrap();
        assert_eq!(summary.trades, 2);
        assert_eq!(summary.depth_snapshots, 2);
        assert_eq!(summary.timestamp_ties, 1);
        let output = fs::read_to_string(&output).unwrap();
        let timestamps: Vec<_> = output
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .map(|event| event["timestamp_ms"].as_i64().unwrap())
            .collect();
        assert_eq!(timestamps, [10, 20, 30, 30]);
        assert!(output.lines().nth(2).unwrap().contains("trade"));
        let _ = fs::remove_file(trades);
        let _ = fs::remove_file(depth);
        let _ = fs::remove_file(temp("merged.jsonl"));
    }

    #[test]
    fn rejects_out_of_order_input_before_writing_a_capture() {
        let trades = temp("bad-trades.jsonl");
        let depth = temp("bad-depth.jsonl");
        let output = temp("bad-merged.jsonl");
        let _ = fs::remove_file(&output);
        fs::write(&trades, "{\"schema_version\":1,\"type\":\"trade\",\"timestamp_ms\":20}\n{\"schema_version\":1,\"type\":\"trade\",\"timestamp_ms\":10}\n").unwrap();
        fs::write(
            &depth,
            "{\"schema_version\":1,\"type\":\"depth\",\"timestamp_ms\":15}\n",
        )
        .unwrap();
        let error = merge(&trades, &depth, &output).unwrap_err();
        assert!(error.contains("moved backwards"));
        assert!(!std::path::Path::new(&output).exists());
        let _ = fs::remove_file(trades);
        let _ = fs::remove_file(depth);
        let _ = fs::remove_file(output);
    }
}
