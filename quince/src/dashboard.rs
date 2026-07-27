// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Read-only local operator dashboard.
//!
//! It deliberately has no order-control endpoints. A dedicated background
//! worker reads the durable journal and delivers snapshots through a bounded
//! crossbeam channel; the engine's latency-sensitive loop never waits on HTTP,
//! a mutex, or a dashboard client.

use axum::{extract::State, http::StatusCode, response::Html, routing::get, Json, Router};
use quince::engine::OrderJournal;
use serde::Serialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize)]
struct Snapshot {
    status: &'static str,
    generated_at_ms: u64,
    journal_path: String,
    records: usize,
    unresolved_client_order_ids: Vec<String>,
    last_error: Option<String>,
}

type Shared = Arc<RwLock<Snapshot>>;

pub fn start(addr: SocketAddr, journal_path: PathBuf) -> Result<(), String> {
    let initial = Snapshot {
        status: "starting",
        generated_at_ms: now_ms(),
        journal_path: journal_path.display().to_string(),
        records: 0,
        unresolved_client_order_ids: Vec::new(),
        last_error: None,
    };
    let state = Arc::new(RwLock::new(initial));
    let (tx, rx) = crossbeam_channel::bounded::<Snapshot>(4);
    std::thread::Builder::new()
        .name("quince-dashboard-journal".into())
        .spawn({
            let journal_path = journal_path.clone();
            move || loop {
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
                    },
                    Err(error) => Snapshot {
                        status: "degraded",
                        generated_at_ms: now_ms(),
                        journal_path: journal_path.display().to_string(),
                        records: 0,
                        unresolved_client_order_ids: Vec::new(),
                        last_error: Some(error.to_string()),
                    },
                };
                let _ = tx.try_send(snapshot);
                std::thread::sleep(Duration::from_secs(1));
            }
        })
        .map_err(|e| format!("start dashboard journal worker: {e}"))?;

    tokio::spawn({
        let state = state.clone();
        async move {
            loop {
                while let Ok(snapshot) = rx.try_recv() {
                    if let Ok(mut current) = state.write() {
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
            .route("/api/v1/state", get(snapshot))
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
        .read()
        .map(|s| Json(s.clone()))
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}

async fn health(State(state): State<Shared>) -> StatusCode {
    match state.read() {
        Ok(snapshot) if snapshot.status == "ok" => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn index() -> Html<&'static str> {
    Html("<!doctype html><title>Quince</title><style>body{font:16px system-ui;background:#101717;color:#d7fff5;max-width:760px;margin:3rem auto}pre{background:#162322;padding:1rem;border-radius:8px}</style><h1>Quince operator dashboard</h1><p>Read-only. The engine never accepts order commands over HTTP.</p><pre id=s>loading…</pre><script>async function x(){let r=await fetch('/api/v1/state');s.textContent=JSON.stringify(await r.json(),null,2)}x();setInterval(x,1000)</script>")
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
