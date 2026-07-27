// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Durable append-only order journal.
//!
//! The journal is deliberately independent from the live order manager.  It
//! records the client-order-id lifecycle before the engine attempts a remote
//! action, allowing a future startup recovery pass to find orders whose
//! submission outcome is unknown.  Each record is one versioned JSON line and
//! is synced before [`OrderJournal::append`] returns.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Current on-disk JSONL schema version.
pub const JOURNAL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalRecord {
    pub version: u32,
    pub sequence: u64,
    pub recorded_at_ms: u64,
    pub event: JournalEvent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JournalEvent {
    /// Persist before attempting `place_order`.
    Registered {
        client_order_id: String,
        symbol: String,
        side: String,
        qty: f64,
        reduce_only: bool,
    },
    /// The exchange returned an order identifier for the client order.
    Accepted {
        client_order_id: String,
        exchange_order_id: String,
    },
    /// A transport error made it unsafe to assume placement failed.
    SubmissionUnknown {
        client_order_id: String,
        error: String,
    },
    /// Cancellation was requested but has not necessarily completed.
    CancelRequested {
        client_order_id: String,
        exchange_order_id: String,
    },
    /// An authoritative exchange status made the order terminal.
    Terminal {
        client_order_id: String,
        status: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("order journal I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON in completed journal record {line}: {source}")]
    Json {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize journal record: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("unsupported journal schema version {version} in record {line}")]
    UnsupportedVersion { line: usize, version: u32 },
    #[error("non-monotonic journal sequence in record {line}: expected {expected}, got {actual}")]
    InvalidSequence {
        line: usize,
        expected: u64,
        actual: u64,
    },
    #[error("system clock is before the Unix epoch")]
    Clock,
}

pub type Result<T> = std::result::Result<T, JournalError>;

/// A single-process writer for a durable order lifecycle journal.
pub struct OrderJournal {
    path: PathBuf,
    file: File,
    next_sequence: u64,
}

impl OrderJournal {
    /// Open (or create) a journal and continue its monotonically increasing
    /// record sequence.  A partial final record is ignored because it can only
    /// result from an interrupted append; all completed records are checked.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let prior = Self::recover(&path)?;
        let next_sequence = prior
            .last()
            .map(|record| record.sequence.saturating_add(1))
            .unwrap_or(0);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            file,
            next_sequence,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append and durably flush one event.  The record is returned so callers
    /// can use its sequence in logs/metrics without reconstructing it.
    pub fn append(&mut self, event: JournalEvent) -> Result<JournalRecord> {
        let recorded_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| JournalError::Clock)?
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        let record = JournalRecord {
            version: JOURNAL_VERSION,
            sequence: self.next_sequence,
            recorded_at_ms,
            event,
        };
        let mut encoded = serde_json::to_vec(&record).map_err(JournalError::Serialize)?;
        encoded.push(b'\n');
        self.file.write_all(&encoded)?;
        self.file.flush()?;
        // `flush` only reaches the OS page cache.  A journal protects against
        // restart ambiguity, so make the durability boundary explicit.
        self.file.sync_data()?;
        self.next_sequence = self.next_sequence.saturating_add(1);
        Ok(record)
    }

    /// Read complete records from a journal.  A final line without a newline
    /// is intentionally ignored: it could be a torn write and must never be
    /// used for recovery.  Any malformed *completed* line is an error.
    pub fn recover(path: impl AsRef<Path>) -> Result<Vec<JournalRecord>> {
        let path = path.as_ref();
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut records = Vec::new();
        let mut reader = BufReader::new(file);
        let mut line = Vec::new();
        let mut line_number = 0;
        let mut expected_sequence = 0;

        loop {
            line.clear();
            let bytes_read = reader.read_until(b'\n', &mut line)?;
            if bytes_read == 0 {
                break;
            }
            line_number += 1;
            if !line.ends_with(b"\n") {
                // Only the final, non-newline-terminated record is ignored.
                break;
            }
            line.pop();
            let record: JournalRecord =
                serde_json::from_slice(&line).map_err(|source| JournalError::Json {
                    line: line_number,
                    source,
                })?;
            if record.version != JOURNAL_VERSION {
                return Err(JournalError::UnsupportedVersion {
                    line: line_number,
                    version: record.version,
                });
            }
            if record.sequence != expected_sequence {
                return Err(JournalError::InvalidSequence {
                    line: line_number,
                    expected: expected_sequence,
                    actual: record.sequence,
                });
            }
            expected_sequence = expected_sequence.saturating_add(1);
            records.push(record);
        }
        Ok(records)
    }

    /// Return client IDs whose last durable state is not terminal.  Startup
    /// uses this to refuse a new trading session until ambiguous orders have
    /// been reconciled, rather than risking duplicate execution after a crash.
    pub fn unresolved_client_order_ids(records: &[JournalRecord]) -> Vec<String> {
        let mut unresolved = BTreeSet::new();
        for record in records {
            match &record.event {
                JournalEvent::Registered {
                    client_order_id, ..
                }
                | JournalEvent::Accepted {
                    client_order_id, ..
                }
                | JournalEvent::SubmissionUnknown {
                    client_order_id, ..
                }
                | JournalEvent::CancelRequested {
                    client_order_id, ..
                } => {
                    unresolved.insert(client_order_id.clone());
                }
                JournalEvent::Terminal {
                    client_order_id, ..
                } => {
                    unresolved.remove(client_order_id);
                }
            }
        }
        unresolved.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("quince-journal-{label}-{nonce}.jsonl"))
    }

    fn registered(id: &str) -> JournalEvent {
        JournalEvent::Registered {
            client_order_id: id.into(),
            symbol: "BTCUSDT".into(),
            side: "buy".into(),
            qty: 1.0,
            reduce_only: false,
        }
    }

    #[test]
    fn append_is_recoverable_and_sequences_continue_after_reopen() {
        let path = temp_path("roundtrip");
        let mut journal = OrderJournal::open(&path).unwrap();
        assert_eq!(journal.append(registered("qc_1")).unwrap().sequence, 0);
        drop(journal);

        let records = OrderJournal::recover(&path).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].version, JOURNAL_VERSION);
        assert_eq!(records[0].event, registered("qc_1"));

        let mut journal = OrderJournal::open(&path).unwrap();
        assert_eq!(journal.append(registered("qc_2")).unwrap().sequence, 1);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn recover_ignores_a_truncated_final_line() {
        let path = temp_path("truncated");
        let mut journal = OrderJournal::open(&path).unwrap();
        journal.append(registered("qc_1")).unwrap();
        drop(journal);
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(br#"{"version":1,"sequence":1"#).unwrap();
        file.flush().unwrap();

        let records = OrderJournal::recover(&path).unwrap();
        assert_eq!(records.len(), 1);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn recover_rejects_malformed_completed_line() {
        let path = temp_path("malformed");
        fs::write(&path, b"this is not json\n").unwrap();

        assert!(matches!(
            OrderJournal::recover(&path),
            Err(JournalError::Json { line: 1, .. })
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn recover_rejects_unknown_schema_version() {
        let path = temp_path("version");
        fs::write(
            &path,
            b"{\"version\":99,\"sequence\":0,\"recorded_at_ms\":0,\"event\":{\"kind\":\"terminal\",\"client_order_id\":\"qc_1\",\"status\":\"FILLED\"}}\n",
        )
        .unwrap();

        assert!(matches!(
            OrderJournal::recover(&path),
            Err(JournalError::UnsupportedVersion {
                line: 1,
                version: 99
            })
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn recover_rejects_non_monotonic_sequences() {
        let path = temp_path("sequence");
        fs::write(
            &path,
            b"{\"version\":1,\"sequence\":3,\"recorded_at_ms\":0,\"event\":{\"kind\":\"terminal\",\"client_order_id\":\"qc_1\",\"status\":\"FILLED\"}}\n",
        )
        .unwrap();

        assert!(matches!(
            OrderJournal::recover(&path),
            Err(JournalError::InvalidSequence {
                line: 1,
                expected: 0,
                actual: 3
            })
        ));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn unresolved_ids_only_include_nonterminal_orders() {
        let records = vec![
            JournalRecord {
                version: JOURNAL_VERSION,
                sequence: 0,
                recorded_at_ms: 0,
                event: registered("qc_done"),
            },
            JournalRecord {
                version: JOURNAL_VERSION,
                sequence: 1,
                recorded_at_ms: 0,
                event: JournalEvent::Terminal {
                    client_order_id: "qc_done".into(),
                    status: "FILLED".into(),
                },
            },
            JournalRecord {
                version: JOURNAL_VERSION,
                sequence: 2,
                recorded_at_ms: 0,
                event: JournalEvent::SubmissionUnknown {
                    client_order_id: "qc_unknown".into(),
                    error: "disconnect".into(),
                },
            },
        ];
        assert_eq!(
            OrderJournal::unresolved_client_order_ids(&records),
            vec!["qc_unknown"]
        );
    }
}
