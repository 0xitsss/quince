//! Lock-free runtime counters exposed to an out-of-band operator surface.

use crate::strategy_lifecycle::{DeploymentMode, StrategyRevision};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const LATENCY_BUCKETS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeTelemetrySnapshot {
    pub strategy_version: u64,
    pub execution_mode: &'static str,
    pub artifact_digest: String,
    pub market_events: u64,
    pub order_intents: u64,
    pub suppressed_orders: u64,
    /// Number of market events whose full engine handling time was sampled.
    pub market_event_latency_samples: u64,
    /// Approximate percentiles, expressed as conservative upper bounds in µs.
    ///
    /// The hot path writes to a fixed log2 histogram with relaxed atomics, so
    /// no allocator, lock, or operator-side backpressure can affect execution.
    pub market_event_latency_p50_us: u64,
    pub market_event_latency_p95_us: u64,
    pub market_event_latency_p99_us: u64,
}

/// Atomic counters only: recording telemetry is safe in the market-data hot
/// path and never waits for an operator client.
#[derive(Debug)]
pub struct RuntimeTelemetry {
    strategy_version: AtomicU64,
    mode: AtomicU8,
    digest_prefix: AtomicU64,
    market_events: AtomicU64,
    order_intents: AtomicU64,
    suppressed_orders: AtomicU64,
    market_event_latency_ns: [AtomicU64; LATENCY_BUCKETS],
}

impl Default for RuntimeTelemetry {
    fn default() -> Self {
        Self {
            strategy_version: AtomicU64::new(0),
            mode: AtomicU8::new(0),
            digest_prefix: AtomicU64::new(0),
            market_events: AtomicU64::new(0),
            order_intents: AtomicU64::new(0),
            suppressed_orders: AtomicU64::new(0),
            market_event_latency_ns: std::array::from_fn(|_| AtomicU64::new(0)),
        }
    }
}

impl RuntimeTelemetry {
    /// Process-wide operator surface. The engine is single-session by design;
    /// this lets a dashboard start before the selected exchange adapter.
    pub fn global() -> Arc<Self> {
        static TELEMETRY: OnceLock<Arc<RuntimeTelemetry>> = OnceLock::new();
        Arc::clone(TELEMETRY.get_or_init(|| Arc::new(Self::default())))
    }
    pub fn set_revision(&self, revision: &StrategyRevision) {
        self.strategy_version
            .store(revision.version, Ordering::Release);
        self.mode.store(
            match revision.mode {
                DeploymentMode::Shadow => 0,
                DeploymentMode::Live => 1,
            },
            Ordering::Release,
        );
        self.digest_prefix.store(
            u64::from_be_bytes(
                revision.artifact_digest[..8]
                    .try_into()
                    .expect("digest prefix"),
            ),
            Ordering::Release,
        );
    }

    pub fn record_market_event(&self) {
        self.market_events.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_order_intent(&self) {
        self.order_intents.fetch_add(1, Ordering::Relaxed);
    }
    pub fn record_suppressed_order(&self) {
        self.suppressed_orders.fetch_add(1, Ordering::Relaxed);
    }

    /// Records complete market-event handling latency, including indicator and
    /// QFL evaluation. Histogram buckets are powers of two nanoseconds.
    pub fn record_market_event_latency(&self, elapsed: Duration) {
        let nanos = elapsed.as_nanos().min(u64::MAX as u128) as u64;
        let bucket = log2_bucket(nanos.max(1));
        self.market_event_latency_ns[bucket].fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> RuntimeTelemetrySnapshot {
        let latency = self.market_event_latency_percentiles();
        RuntimeTelemetrySnapshot {
            strategy_version: self.strategy_version.load(Ordering::Acquire),
            execution_mode: if self.mode.load(Ordering::Acquire) == 0 {
                "shadow"
            } else {
                "live"
            },
            artifact_digest: format!("{:016x}", self.digest_prefix.load(Ordering::Acquire)),
            market_events: self.market_events.load(Ordering::Relaxed),
            order_intents: self.order_intents.load(Ordering::Relaxed),
            suppressed_orders: self.suppressed_orders.load(Ordering::Relaxed),
            market_event_latency_samples: latency.samples,
            market_event_latency_p50_us: latency.p50_us,
            market_event_latency_p95_us: latency.p95_us,
            market_event_latency_p99_us: latency.p99_us,
        }
    }

    fn market_event_latency_percentiles(&self) -> LatencyPercentiles {
        let buckets: Vec<u64> = self
            .market_event_latency_ns
            .iter()
            .map(|bucket| bucket.load(Ordering::Relaxed))
            .collect();
        let samples = buckets.iter().sum();
        LatencyPercentiles {
            samples,
            p50_us: percentile_upper_bound_us(&buckets, samples, 50),
            p95_us: percentile_upper_bound_us(&buckets, samples, 95),
            p99_us: percentile_upper_bound_us(&buckets, samples, 99),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LatencyPercentiles {
    samples: u64,
    p50_us: u64,
    p95_us: u64,
    p99_us: u64,
}

fn log2_bucket(nanos: u64) -> usize {
    (u64::BITS as usize - 1 - nanos.leading_zeros() as usize).min(LATENCY_BUCKETS - 1)
}

fn percentile_upper_bound_us(buckets: &[u64], samples: u64, percentile: u64) -> u64 {
    if samples == 0 {
        return 0;
    }
    // Nearest-rank quantile, then the bucket's inclusive upper bound. This is
    // intentionally conservative: operators never see a percentile below the
    // latency range recorded by the fixed histogram.
    let target = samples.saturating_mul(percentile).saturating_add(99) / 100;
    let mut cumulative: u64 = 0;
    for (index, count) in buckets.iter().enumerate() {
        cumulative = cumulative.saturating_add(*count);
        if cumulative >= target {
            let upper_ns = 1u64
                .checked_shl((index + 1) as u32)
                .unwrap_or(u64::MAX)
                .saturating_sub(1);
            return upper_ns.saturating_add(999) / 1_000;
        }
    }
    u64::MAX / 1_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_tracks_revision_and_hot_path_counters() {
        let telemetry = RuntimeTelemetry::default();
        telemetry.set_revision(&StrategyRevision::new(
            7,
            [0xab; 32],
            DeploymentMode::Shadow,
        ));
        telemetry.record_market_event();
        telemetry.record_order_intent();
        telemetry.record_suppressed_order();
        assert_eq!(
            telemetry.snapshot(),
            RuntimeTelemetrySnapshot {
                strategy_version: 7,
                execution_mode: "shadow",
                artifact_digest: "abababababababab".into(),
                market_events: 1,
                order_intents: 1,
                suppressed_orders: 1,
                market_event_latency_samples: 0,
                market_event_latency_p50_us: 0,
                market_event_latency_p95_us: 0,
                market_event_latency_p99_us: 0,
            }
        );
    }

    #[test]
    fn latency_percentiles_are_lock_free_histogram_upper_bounds() {
        let telemetry = RuntimeTelemetry::default();
        for _ in 0..90 {
            telemetry.record_market_event_latency(Duration::from_nanos(1_000));
        }
        for _ in 0..9 {
            telemetry.record_market_event_latency(Duration::from_nanos(8_000));
        }
        telemetry.record_market_event_latency(Duration::from_nanos(1_000_000));

        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.market_event_latency_samples, 100);
        // Bucket bounds are powers of two, rounded up to µs.
        assert_eq!(snapshot.market_event_latency_p50_us, 2);
        assert_eq!(snapshot.market_event_latency_p95_us, 9);
        assert_eq!(snapshot.market_event_latency_p99_us, 9);
    }
}
// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
