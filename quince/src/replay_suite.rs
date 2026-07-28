// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Deterministic batch replay reporting.
//!
//! A suite never turns a failed/unsupported strategy into a zero-result run.
//! Every discovered artifact has a corresponding outcome, so an operator can
//! distinguish a strategy that produced no intents from one that did not load.

use crate::replay::{self, ReplaySummary};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum ReplaySuiteError {
    #[error("read strategy directory {path}: {source}")]
    ReadDirectory {
        path: String,
        source: std::io::Error,
    },
    #[error("read strategy directory entry in {path}: {source}")]
    ReadDirectoryEntry {
        path: String,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplaySuiteResult {
    pub strategy: String,
    /// `ok` means the artifact loaded and consumed the capture. `error` means
    /// no result may be inferred from this entry.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ReplaySummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplaySuiteSummary {
    pub schema_version: u8,
    pub capture: String,
    pub symbol: String,
    pub strategies_discovered: u64,
    pub strategies_succeeded: u64,
    pub strategies_failed: u64,
    pub results: Vec<ReplaySuiteResult>,
}

/// Run every immediate `.qfl` artifact in `strategy_directory` in a
/// stable lexical order. The capture is replayed separately for every strategy
/// so state can never leak between artifacts.
pub fn run(
    strategy_directory: &str,
    capture_path: &str,
    symbol: &str,
) -> Result<ReplaySuiteSummary, ReplaySuiteError> {
    let mut artifacts = discover(strategy_directory)?;
    artifacts.sort();

    let mut results = Vec::with_capacity(artifacts.len());
    let mut succeeded = 0_u64;
    for artifact in artifacts {
        let strategy = artifact.to_string_lossy().into_owned();
        match replay::run(&strategy, capture_path, symbol) {
            Ok(summary) => {
                succeeded += 1;
                results.push(ReplaySuiteResult {
                    strategy,
                    status: "ok".into(),
                    summary: Some(summary),
                    error: None,
                });
            }
            Err(error) => results.push(ReplaySuiteResult {
                strategy,
                status: "error".into(),
                summary: None,
                error: Some(error.to_string()),
            }),
        }
    }

    Ok(ReplaySuiteSummary {
        schema_version: 1,
        capture: capture_path.into(),
        symbol: symbol.into(),
        strategies_discovered: results.len() as u64,
        strategies_succeeded: succeeded,
        strategies_failed: results.len() as u64 - succeeded,
        results,
    })
}

fn discover(strategy_directory: &str) -> Result<Vec<PathBuf>, ReplaySuiteError> {
    let entries = std::fs::read_dir(strategy_directory).map_err(|source| {
        ReplaySuiteError::ReadDirectory {
            path: strategy_directory.into(),
            source,
        }
    })?;
    let mut artifacts = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ReplaySuiteError::ReadDirectoryEntry {
            path: strategy_directory.into(),
            source,
        })?;
        let path = entry.path();
        if is_strategy_artifact(&path) {
            artifacts.push(path);
        }
    }
    Ok(artifacts)
}

fn is_strategy_artifact(path: &Path) -> bool {
    path.is_file()
        && matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("qfl")
        )
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
            std::env::temp_dir().join(format!("quince-replay-suite-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn suite_reports_success_and_compile_errors_without_hiding_them() {
        let directory = fixture_dir();
        let capture = directory.join("capture.jsonl");
        fs::write(
            &capture,
            r#"{"schema_version":1,"type":"eval","timestamp_ms":1700000000000}"#,
        )
        .unwrap();
        fs::write(
            directory.join("01-valid.qfl"),
            "on eval() { quince.log(\"signal\") }",
        )
        .unwrap();
        fs::write(directory.join("02-invalid.qfl"), "on eval( {").unwrap();
        fs::write(directory.join("notes.txt"), "not a strategy").unwrap();

        let suite = run(
            directory.to_str().unwrap(),
            capture.to_str().unwrap(),
            "BTCUSDT",
        )
        .unwrap();
        assert_eq!(suite.strategies_discovered, 2);
        assert_eq!(suite.strategies_succeeded, 1);
        assert_eq!(suite.strategies_failed, 1);
        assert_eq!(suite.results[0].status, "ok");
        assert_eq!(suite.results[1].status, "error");
        assert!(suite.results[1].summary.is_none());
        assert!(suite.results[1]
            .error
            .as_deref()
            .unwrap()
            .contains("load strategy"));
        fs::remove_dir_all(directory).unwrap();
    }
}
