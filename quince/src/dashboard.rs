// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Read-only local operator dashboard.
//!
//! It deliberately has no order-control endpoints. A dedicated background
//! worker reads the durable journal and delivers snapshots through a bounded
//! crossbeam channel; the engine's latency-sensitive loop never waits on HTTP,
//! a mutex, or a dashboard client.

use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use quince::engine::{
    OrderJournal, RuntimeTelemetry, RuntimeTelemetrySnapshot, StrategyControlCommand,
    StrategyControlError, StrategyControlSender,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_SNAPSHOT_AGE_MS: u64 = 3_000;

#[derive(Clone, Debug, Serialize)]
struct Snapshot {
    status: &'static str,
    generated_at_ms: u64,
    journal_path: String,
    records: usize,
    unresolved_client_order_ids: Vec<String>,
    last_error: Option<String>,
    journal_refresh_latency_us: u64,
    execution: RuntimeTelemetrySnapshot,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct Metrics {
    snapshot_age_ms: u64,
    journal_refresh_latency_us: u64,
    journal_queue_depth: usize,
    journal_worker_dropped_snapshots: u64,
    market_events: u64,
    order_intents: u64,
    suppressed_orders: u64,
}

struct DashboardState {
    snapshot: RwLock<Snapshot>,
    journal_queue_depth: AtomicUsize,
    journal_worker_dropped_snapshots: AtomicU64,
    telemetry: std::sync::Arc<RuntimeTelemetry>,
}

type Shared = Arc<DashboardState>;

/// A lifecycle request accepted by the control-plane transport.
///
/// This remains a proposal: the engine owns the matching receiver, validates
/// lifecycle state, and appends the terminal audit result.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)] // Opt-in router; deliberately absent from the default dashboard.
pub(crate) enum ControlAction {
    PromoteShadow,
    Rollback,
    DemoteToShadow,
    PauseExecution,
    ResumeExecution,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[allow(dead_code)] // Constructed by Axum only when an operator opts in.
struct ControlRequest {
    action: ControlAction,
    requested_by: String,
    #[serde(default)]
    reason: Option<String>,
}

#[allow(dead_code)] // Reached only by the opt-in Axum handler.
fn command_for(request: &ControlRequest) -> Result<StrategyControlCommand, String> {
    let command = match request.action {
        ControlAction::PromoteShadow => StrategyControlCommand::PromoteShadow,
        ControlAction::Rollback => StrategyControlCommand::Rollback,
        ControlAction::DemoteToShadow => StrategyControlCommand::DemoteToShadow,
        ControlAction::PauseExecution => {
            let reason = request
                .reason
                .as_deref()
                .map(str::trim)
                .filter(|reason| !reason.is_empty() && reason.len() <= 256)
                .ok_or_else(|| "pause_execution requires a 1..=256 byte reason".to_owned())?;
            return Ok(StrategyControlCommand::PauseExecution {
                reason: reason.to_owned(),
            });
        }
        ControlAction::ResumeExecution => StrategyControlCommand::ResumeExecution,
    };
    if request.reason.is_some() {
        return Err("reason is only accepted for pause_execution".into());
    }
    Ok(command)
}

#[derive(Debug, Serialize)]
#[allow(dead_code)] // Serialized by the opt-in Axum handler.
struct ControlAccepted {
    request_id: u64,
    status: &'static str,
}

#[derive(Debug, Serialize)]
#[allow(dead_code)] // Serialized by the opt-in Axum handler.
struct ControlRejected {
    error: String,
}

/// Returns an opt-in control router. The default dashboard stays read-only.
/// Callers must pass an engine-created sender: the transport has no mutable
/// VM, journal, exchange, or receiver access.
#[allow(dead_code)] // Enabled only by a future explicit control-plane bootstrap.
pub(crate) fn control_router(sender: StrategyControlSender) -> Router {
    Router::new()
        .route("/api/v1/control/commands", post(submit_control_command))
        .layer(DefaultBodyLimit::max(4 * 1024))
        .with_state(sender)
}

#[allow(dead_code)] // Registered only by `control_router`.
async fn submit_control_command(
    State(sender): State<StrategyControlSender>,
    Json(request): Json<ControlRequest>,
) -> Result<(StatusCode, Json<ControlAccepted>), (StatusCode, Json<ControlRejected>)> {
    let command = command_for(&request)
        .map_err(|error| (StatusCode::BAD_REQUEST, Json(ControlRejected { error })))?;
    match sender.try_submit(request.requested_by, command) {
        Ok(request_id) => Ok((
            StatusCode::ACCEPTED,
            Json(ControlAccepted {
                request_id,
                status: "queued",
            }),
        )),
        Err(StrategyControlError::InvalidActor) => Err((
            StatusCode::BAD_REQUEST,
            Json(ControlRejected {
                error: "requested_by must be a non-empty operator identity".into(),
            }),
        )),
        Err(StrategyControlError::QueueFull) => Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(ControlRejected {
                error: "control queue is full; command was not accepted".into(),
            }),
        )),
        Err(StrategyControlError::Disconnected) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ControlRejected {
                error: "control loop is unavailable; command was not accepted".into(),
            }),
        )),
        Err(error) => Err((
            StatusCode::BAD_REQUEST,
            Json(ControlRejected {
                error: error.to_string(),
            }),
        )),
    }
}

pub fn start(addr: SocketAddr, journal_path: PathBuf) -> Result<(), String> {
    let telemetry = RuntimeTelemetry::global();
    let initial = Snapshot {
        status: "starting",
        generated_at_ms: now_ms(),
        journal_path: journal_path.display().to_string(),
        records: 0,
        unresolved_client_order_ids: Vec::new(),
        last_error: None,
        journal_refresh_latency_us: 0,
        execution: telemetry.snapshot(),
    };
    let state = Arc::new(DashboardState {
        snapshot: RwLock::new(initial),
        journal_queue_depth: AtomicUsize::new(0),
        journal_worker_dropped_snapshots: AtomicU64::new(0),
        telemetry,
    });
    let (tx, rx) = crossbeam_channel::bounded::<Snapshot>(4);
    let worker_state = Arc::clone(&state);
    std::thread::Builder::new()
        .name("quince-dashboard-journal".into())
        .spawn({
            let journal_path = journal_path.clone();
            move || loop {
                let started = Instant::now();
                let snapshot = match OrderJournal::recover(&journal_path) {
                    Ok(records) => Snapshot {
                        status: "ok",
                        generated_at_ms: now_ms(),
                        journal_path: journal_path.display().to_string(),
                        unresolved_client_order_ids: OrderJournal::unresolved_client_order_ids(
                            &records,
                        ),
                        records: records.len(),
                        last_error: None,
                        journal_refresh_latency_us: elapsed_us(started),
                        execution: worker_state.telemetry.snapshot(),
                    },
                    Err(error) => Snapshot {
                        status: "degraded",
                        generated_at_ms: now_ms(),
                        journal_path: journal_path.display().to_string(),
                        records: 0,
                        unresolved_client_order_ids: Vec::new(),
                        last_error: Some(error.to_string()),
                        journal_refresh_latency_us: elapsed_us(started),
                        execution: worker_state.telemetry.snapshot(),
                    },
                };
                // Reserve the metric slot before publishing. `try_send` can
                // make the snapshot observable immediately, so incrementing
                // afterwards could race the consumer's decrement and wrap.
                worker_state
                    .journal_queue_depth
                    .fetch_add(1, Ordering::Release);
                match tx.try_send(snapshot) {
                    Ok(()) => {}
                    Err(crossbeam_channel::TrySendError::Full(_)) => {
                        worker_state
                            .journal_queue_depth
                            .fetch_sub(1, Ordering::AcqRel);
                        worker_state
                            .journal_worker_dropped_snapshots
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    Err(crossbeam_channel::TrySendError::Disconnected(_)) => {
                        worker_state
                            .journal_queue_depth
                            .fetch_sub(1, Ordering::AcqRel);
                        break;
                    }
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .map_err(|e| format!("start dashboard journal worker: {e}"))?;

    tokio::spawn({
        let state = state.clone();
        async move {
            loop {
                while let Ok(snapshot) = rx.try_recv() {
                    state.journal_queue_depth.fetch_sub(1, Ordering::AcqRel);
                    if let Ok(mut current) = state.snapshot.write() {
                        *current = snapshot;
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    });
    tokio::spawn(async move {
        let app = Router::new()
            .route("/", get(index))
            .route("/healthz", get(health))
            .route("/readyz", get(readiness))
            .route("/api/v1/state", get(snapshot))
            .route("/api/v1/metrics", get(metrics))
            .with_state(state);
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                tracing::info!(%addr, "read-only Quince dashboard listening");
                if let Err(error) = axum::serve(listener, app).await {
                    tracing::error!(%error, "dashboard server stopped");
                }
            }
            Err(error) => tracing::error!(%error, %addr, "dashboard bind failed"),
        }
    });
    Ok(())
}

async fn snapshot(State(state): State<Shared>) -> Result<Json<Snapshot>, StatusCode> {
    state
        .snapshot
        .read()
        .map(|s| {
            let mut snapshot = s.clone();
            snapshot.execution = state.telemetry.snapshot();
            Json(snapshot)
        })
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

async fn health(State(state): State<Shared>) -> StatusCode {
    if state.snapshot.read().is_ok() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn readiness(State(state): State<Shared>) -> StatusCode {
    match state.snapshot.read() {
        Ok(snapshot)
            if readiness_status(&snapshot, now_ms())
                && state.telemetry.snapshot().execution_sync_ready =>
        {
            StatusCode::OK
        }
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn metrics(State(state): State<Shared>) -> Result<Json<Metrics>, StatusCode> {
    state
        .snapshot
        .read()
        .map(|snapshot| {
            Json(snapshot_metrics(
                &snapshot,
                now_ms(),
                state.journal_queue_depth.load(Ordering::Acquire),
                state
                    .journal_worker_dropped_snapshots
                    .load(Ordering::Relaxed),
                state.telemetry.snapshot(),
            ))
        })
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

async fn index() -> Html<&'static str> {
    Html(
        "<!doctype html><title>Quince</title><style>body{font:16px system-ui;background:#101717;color:#d7fff5;max-width:760px;margin:3rem auto}pre{background:#162322;padding:1rem;border-radius:8px}</style><h1>Quince operator dashboard</h1><p>Read-only. The engine never accepts order commands over HTTP.</p><pre id=s>loading…</pre><script>async function x(){let r=await fetch('/api/v1/state');s.textContent=JSON.stringify(await r.json(),null,2)}x();setInterval(x,1000)</script>",
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn elapsed_us(started: Instant) -> u64 {
    started.elapsed().as_micros().try_into().unwrap_or(u64::MAX)
}

fn readiness_status(snapshot: &Snapshot, now: u64) -> bool {
    snapshot.status == "ok"
        && now.saturating_sub(snapshot.generated_at_ms) <= MAX_SNAPSHOT_AGE_MS
        && snapshot.unresolved_client_order_ids.is_empty()
}

fn snapshot_metrics(
    snapshot: &Snapshot,
    now: u64,
    queue_depth: usize,
    dropped: u64,
    telemetry: RuntimeTelemetrySnapshot,
) -> Metrics {
    Metrics {
        snapshot_age_ms: now.saturating_sub(snapshot.generated_at_ms),
        journal_refresh_latency_us: snapshot.journal_refresh_latency_us,
        journal_queue_depth: queue_depth,
        journal_worker_dropped_snapshots: dropped,
        market_events: telemetry.market_events,
        order_intents: telemetry.order_intents,
        suppressed_orders: telemetry.suppressed_orders,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(status: &'static str, generated_at_ms: u64, unresolved: Vec<&str>) -> Snapshot {
        Snapshot {
            status,
            generated_at_ms,
            journal_path: "orders.jsonl".into(),
            records: 0,
            unresolved_client_order_ids: unresolved.into_iter().map(str::to_owned).collect(),
            last_error: None,
            journal_refresh_latency_us: 1,
            execution: RuntimeTelemetry::global().snapshot(),
        }
    }

    #[test]
    fn readiness_requires_a_fresh_healthy_and_reconciled_snapshot() {
        assert!(readiness_status(&snapshot("ok", 10_000, vec![]), 13_000));
        assert!(!readiness_status(
            &snapshot("starting", 10_000, vec![]),
            10_000
        ));
        assert!(!readiness_status(
            &snapshot("ok", 10_000, vec!["client-1"]),
            10_000
        ));
        assert!(!readiness_status(&snapshot("ok", 10_000, vec![]), 13_001));
    }

    #[test]
    fn readiness_tolerates_clock_regression_without_marking_fresh_data_stale() {
        assert!(readiness_status(&snapshot("ok", 10_000, vec![]), 9_000));
    }

    #[test]
    fn metrics_expose_snapshot_latency_and_backpressure_without_clock_underflow() {
        let metrics = snapshot_metrics(
            &snapshot("ok", 10_000, vec![]),
            9_000,
            3,
            7,
            RuntimeTelemetry::global().snapshot(),
        );
        assert_eq!(
            metrics,
            Metrics {
                snapshot_age_ms: 0,
                journal_refresh_latency_us: 1,
                journal_queue_depth: 3,
                journal_worker_dropped_snapshots: 7,
                market_events: 0,
                order_intents: 0,
                suppressed_orders: 0,
            }
        );
    }

    fn request(action: ControlAction, requested_by: &str) -> ControlRequest {
        ControlRequest {
            action,
            requested_by: requested_by.into(),
            reason: None,
        }
    }

    #[tokio::test]
    async fn control_transport_queues_an_auditable_engine_command_without_mutating_engine() {
        let (sender, receiver) = quince::engine::strategy_control_channel(1, 8).unwrap();
        let (status, response) = submit_control_command(
            State(sender),
            Json(request(ControlAction::PromoteShadow, "operator-42")),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::ACCEPTED);
        assert_eq!(response.0.request_id, 1);
        assert_eq!(response.0.status, "queued");

        let queued = receiver.try_recv().unwrap();
        assert_eq!(queued.requested_by, "operator-42");
        assert_eq!(queued.command, StrategyControlCommand::PromoteShadow);
    }

    #[tokio::test]
    async fn control_transport_rejects_invalid_actors_and_never_enqueues_them() {
        let (sender, receiver) = quince::engine::strategy_control_channel(1, 8).unwrap();
        let rejected =
            submit_control_command(State(sender), Json(request(ControlAction::Rollback, "  ")))
                .await
                .unwrap_err();
        assert_eq!(rejected.0, StatusCode::BAD_REQUEST);
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn control_transport_fails_closed_when_the_bounded_engine_queue_is_full() {
        let (sender, _receiver) = quince::engine::strategy_control_channel(1, 8).unwrap();
        sender
            .try_submit("operator-1", StrategyControlCommand::Rollback)
            .unwrap();
        let rejected = submit_control_command(
            State(sender),
            Json(request(ControlAction::DemoteToShadow, "operator-2")),
        )
        .await
        .unwrap_err();
        assert_eq!(rejected.0, StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn pause_is_the_only_transport_command_that_accepts_a_bounded_reason() {
        let mut pause = request(ControlAction::PauseExecution, "operator-42");
        pause.reason = Some("market-data integrity breach".into());
        assert_eq!(
            command_for(&pause).unwrap(),
            StrategyControlCommand::PauseExecution {
                reason: "market-data integrity breach".into(),
            }
        );
        assert!(command_for(&request(ControlAction::PauseExecution, "operator-42")).is_err());

        let mut resume = request(ControlAction::ResumeExecution, "operator-42");
        resume.reason = Some("ignored reason".into());
        assert!(command_for(&resume).is_err());
    }
}
