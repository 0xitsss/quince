// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Reproducible offline research reports built on deterministic replay.

use crate::replay_suite::{self, ReplaySuiteResult};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ResearchError {
    #[error(transparent)]
    ReplaySuite(#[from] replay_suite::ReplaySuiteError),
    #[error("create research output directory {path}: {source}")]
    CreateDirectory {
        path: String,
        source: std::io::Error,
    },
    #[error("serialize research report: {0}")]
    Serialize(serde_json::Error),
    #[error("write research report {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
}

/// Stable, machine-readable outcome of replaying a strategy set on one capture.
#[derive(Debug, Clone, Serialize)]
pub struct ResearchReport {
    pub schema_version: u8,
    pub capture: String,
    pub symbol: String,
    pub strategies_discovered: u64,
    pub strategies_succeeded: u64,
    pub strategies_failed: u64,
    pub results: Vec<ReplaySuiteResult>,
}

/// Run the replay suite and atomically materialize JSON and self-contained HTML
/// under `output_directory`. The report has no wall-clock timestamp so equal
/// inputs produce byte-for-byte equal JSON.
pub fn write_report(
    strategy_directory: &str,
    capture_path: &str,
    symbol: &str,
    output_directory: &str,
) -> Result<ResearchReport, ResearchError> {
    let suite = replay_suite::run(strategy_directory, capture_path, symbol)?;
    let report = ResearchReport {
        schema_version: 1,
        capture: suite.capture,
        symbol: suite.symbol,
        strategies_discovered: suite.strategies_discovered,
        strategies_succeeded: suite.strategies_succeeded,
        strategies_failed: suite.strategies_failed,
        results: suite.results,
    };
    let output = Path::new(output_directory);
    std::fs::create_dir_all(output).map_err(|source| ResearchError::CreateDirectory {
        path: output.display().to_string(),
        source,
    })?;
    write_json(output.join("research-report.json"), &report)?;
    write_html(output.join("research-report.html"), &report)?;
    Ok(report)
}

fn write_json(path: PathBuf, report: &ResearchReport) -> Result<(), ResearchError> {
    let json = serde_json::to_vec_pretty(report).map_err(ResearchError::Serialize)?;
    std::fs::write(&path, json).map_err(|source| ResearchError::Write {
        path: path.display().to_string(),
        source,
    })
}

fn write_html(path: PathBuf, report: &ResearchReport) -> Result<(), ResearchError> {
    std::fs::write(&path, render_html(report)).map_err(|source| ResearchError::Write {
        path: path.display().to_string(),
        source,
    })
}

fn render_html(report: &ResearchReport) -> String {
    let rows = report
        .results
        .iter()
        .map(|result| match &result.summary {
            Some(summary) => format!(
                "<tr><td>{}</td><td>ok</td><td>{:.4}%</td><td>{:.4}%</td><td>{:.4}</td><td>{:.4}</td><td>{}/{}</td><td>{:.8}</td></tr>",
                html_escape(&result.strategy),
                summary.performance.net_return_fraction * 100.0,
                summary.performance.max_drawdown_fraction * 100.0,
                summary.performance.sharpe_per_observation.unwrap_or(0.0),
                summary.performance.sortino_per_observation.unwrap_or(0.0),
                summary.paper_fills,
                summary.order_intents,
                summary.net_pnl_quote,
            ),
            None => format!(
                "<tr><td>{}</td><td>error</td><td colspan=6>{}</td></tr>",
                html_escape(&result.strategy),
                html_escape(result.error.as_deref().unwrap_or("unknown replay failure")),
            ),
        })
        .collect::<String>();
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Quince Research Report</title><style>body{{background:#101516;color:#e8f0ef;font:15px system-ui,sans-serif;margin:2rem}}table{{border-collapse:collapse;width:100%;background:#172120}}th,td{{border:1px solid #38504d;padding:.65rem;text-align:right}}th:first-child,td:first-child,td[colspan]{{text-align:left}}th{{color:#79f0d3}}code{{color:#a8ffef}}</style></head><body><h1>Quince Research Report</h1><p>Capture: <code>{}</code><br>Symbol: <code>{}</code><br>Strategies: {} succeeded / {} discovered</p><p>Sharpe and Sortino are <strong>per-observation</strong>, not annualized: replay events have irregular timing. Validate candidates out-of-sample with the paired JSON before promotion.</p><table><thead><tr><th>Strategy</th><th>Status</th><th>Net return</th><th>Max DD</th><th>Sharpe/event</th><th>Sortino/event</th><th>Fills/intents</th><th>Net PnL</th></tr></thead><tbody>{rows}</tbody></table></body></html>",
        html_escape(&report.capture),
        html_escape(&report.symbol),
        report.strategies_succeeded,
        report.strategies_discovered,
    )
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    fn fixture_dir() -> PathBuf {
        let id = FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("quince-research-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn html_escape_never_interprets_strategy_or_capture_text() {
        assert_eq!(
            html_escape("<script>'\"&"),
            "&lt;script&gt;&#39;&quot;&amp;"
        );
    }

    #[test]
    fn report_writes_json_and_html_for_a_strategy_directory() {
        let directory = fixture_dir();
        let strategies = directory.join("strategies");
        let output = directory.join("report");
        fs::create_dir_all(&strategies).unwrap();
        fs::write(
            strategies.join("signal.qfl"),
            "on eval() { quince.log(\"signal\") }",
        )
        .unwrap();
        let capture = directory.join("capture.jsonl");
        fs::write(
            &capture,
            r#"{"schema_version":1,"type":"eval","timestamp_ms":1700000000000}"#,
        )
        .unwrap();

        let report = write_report(
            strategies.to_str().unwrap(),
            capture.to_str().unwrap(),
            "BTCUSDT",
            output.to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(report.strategies_succeeded, 1);
        let json = fs::read_to_string(output.join("research-report.json")).unwrap();
        assert!(json.contains("max_drawdown_fraction"));
        let html = fs::read_to_string(output.join("research-report.html")).unwrap();
        assert!(html.contains("Quince Research Report"));
        fs::remove_dir_all(directory).unwrap();
    }
}
