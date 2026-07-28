// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Quince trading bot binary entry point.
//! Configures the trading environment from environment variables, selects
//! mock/public/live exchange mode, and launches the main engine event loop.

mod capture_merge;
mod dashboard;
mod mock;
mod okx_import;
mod replay;
mod replay_suite;
mod wallet;

use quince::engine::{Engine, EngineError, OrderJournal};
use quince::exchange::r#trait::Exchange;
use quince::qfl::config::{load_strategy_config, ExchangeKind, Network};
use quince::qfl::runtime::QflRuntime;
use quince::risk::{RiskConfig, RiskControls};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

fn start_dashboard_if_enabled(log_path: &str) -> Result<(), EngineError> {
    if !env_flag("QUINCE_DASHBOARD")? {
        return Ok(());
    }
    let addr = std::env::var("QUINCE_DASHBOARD_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:3000".into())
        .parse()
        .map_err(|e| EngineError::Strategy(format!("invalid QUINCE_DASHBOARD_ADDR: {e}")))?;
    dashboard::start(
        addr,
        std::path::Path::new(log_path).with_extension("orders.jsonl"),
    )
    .map_err(EngineError::Strategy)
}

/// Offline recovery tooling intentionally lives before runtime initialization:
/// it opens no exchange socket and never has access to API credentials.
fn run_operator_command() -> Result<bool, EngineError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return Ok(false);
    }
    if args.as_slice() == ["preflight"] {
        run_preflight()?;
        return Ok(true);
    }
    if let [command, symbol, output] = args.as_slice() {
        if command == "import-okx-depth" {
            let snapshots =
                okx_import::import_snapshot_25(symbol, output).map_err(EngineError::Strategy)?;
            println!(
                "{}",
                serde_json::json!({"status":"ok","snapshots":snapshots,"output":output,"symbol":symbol})
            );
            return Ok(true);
        }
        if command == "import-okx-trades" {
            let trades =
                okx_import::import_trades(symbol, output).map_err(EngineError::Strategy)?;
            println!(
                "{}",
                serde_json::json!({"status":"ok","trades":trades,"output":output,"symbol":symbol})
            );
            return Ok(true);
        }
    }
    if let [command, trades, depth, output] = args.as_slice() {
        if command == "merge-captures" {
            let summary =
                capture_merge::merge(trades, depth, output).map_err(EngineError::Strategy)?;
            println!(
                "{}",
                serde_json::to_string(&summary).map_err(|error| EngineError::Strategy(format!(
                    "serialize capture merge summary: {error}"
                )))?
            );
            return Ok(true);
        }
    }
    if let [command, strategy, capture] = args.as_slice() {
        if command == "replay" {
            let symbol = std::env::var("QUINCE_SYMBOL").unwrap_or_else(|_| "BTCUSDT".into());
            let summary = replay::run(strategy, capture, &symbol)
                .map_err(|error| EngineError::Strategy(error.to_string()))?;
            println!(
                "{}",
                serde_json::to_string(&summary).map_err(|error| EngineError::Strategy(format!(
                    "serialize replay summary: {error}"
                )))?
            );
            return Ok(true);
        }
    }
    if let [command, directory, capture] = args.as_slice() {
        if command == "replay-suite" {
            let symbol = std::env::var("QUINCE_SYMBOL").unwrap_or_else(|_| "BTCUSDT".into());
            let summary = replay_suite::run(directory, capture, &symbol)
                .map_err(|error| EngineError::Strategy(error.to_string()))?;
            println!(
                "{}",
                serde_json::to_string(&summary).map_err(|error| EngineError::Strategy(format!(
                    "serialize replay suite summary: {error}"
                )))?
            );
            return Ok(true);
        }
    }
    let [group, action, path] = args.as_slice() else {
        return Err(EngineError::Strategy(
            "usage: quince preflight | quince import-okx-depth <symbol> <capture.jsonl> | quince import-okx-trades <symbol> <capture.jsonl> | quince merge-captures <trades.jsonl> <depth.jsonl> <capture.jsonl> | quince replay <strategy.qfl|qfr> <capture.jsonl> | quince replay-suite <strategy-directory> <capture.jsonl> | quince journal <inspect|verify> <orders.jsonl>".into(),
        ));
    };
    if group != "journal" || !matches!(action.as_str(), "inspect" | "verify") {
        return Err(EngineError::Strategy(
            "usage: quince preflight | quince import-okx-depth <symbol> <capture.jsonl> | quince import-okx-trades <symbol> <capture.jsonl> | quince merge-captures <trades.jsonl> <depth.jsonl> <capture.jsonl> | quince replay <strategy.qfl|qfr> <capture.jsonl> | quince replay-suite <strategy-directory> <capture.jsonl> | quince journal <inspect|verify> <orders.jsonl>".into(),
        ));
    }
    let records = OrderJournal::recover(path)?;
    let unresolved = OrderJournal::unresolved_client_order_ids(&records);
    if action == "inspect" {
        println!(
            "{}",
            serde_json::json!({
                "journal": path,
                "records": records.len(),
                "unresolved_client_order_ids": unresolved,
                "safe_to_start": unresolved.is_empty(),
            })
        );
    } else if unresolved.is_empty() {
        println!(
            "journal verified: {} records, no unresolved orders",
            records.len()
        );
    } else {
        return Err(EngineError::Strategy(format!(
            "journal has unresolved client order IDs: {}; reconcile them against the exchange before starting",
            unresolved.join(", ")
        )));
    }
    Ok(true)
}

/// Validate a launch without opening sockets, loading credentials, or creating
/// any order artifacts. Intended for CI and operator runbooks.
fn run_preflight() -> Result<(), EngineError> {
    let is_mock = env_flag("QUINCE_MOCK")?;
    let is_public = env_flag("QUINCE_PUBLIC")?;
    let live_enabled = env_flag("QUINCE_LIVE")?;
    let shadow_enabled = env_flag("QUINCE_SHADOW")?;
    validate_runtime_modes(is_mock, is_public, live_enabled)?;

    let strategy =
        std::env::var("QUINCE_STRATEGY").unwrap_or_else(|_| "strategies/test_all.qfl".into());
    let symbol = std::env::var("QUINCE_SYMBOL").unwrap_or_else(|_| "btcusdt".into());
    if symbol.trim().is_empty() {
        return Err(EngineError::Strategy(
            "QUINCE_SYMBOL must not be empty".into(),
        ));
    }
    let mut config = load_strategy_config(&strategy).map_err(EngineError::Strategy)?;
    if let Ok(exchange) = std::env::var("QUINCE_EXCHANGE") {
        config.exchange = ExchangeKind::parse(&exchange).map_err(EngineError::Strategy)?;
    }
    if let Ok(network) = std::env::var("QUINCE_NETWORK") {
        config.network = Network::parse(&network).map_err(EngineError::Strategy)?;
    }
    if strategy.ends_with(".qfr") {
        QflRuntime::load_qfr(&strategy).map_err(EngineError::Strategy)?;
    } else {
        QflRuntime::load(&strategy).map_err(EngineError::Strategy)?;
    }

    let max_pos = env_f64("QUINCE_MAX_POSITION", 1.0)?;
    let max_dd = env_f64("QUINCE_MAX_DRAWDOWN", 0.05)?;
    let max_freq = env_u32("QUINCE_MAX_ORDER_FREQ", 10)?;
    let max_loss = env_f64("QUINCE_MAX_DAILY_LOSS", 1000.0)?;
    let max_market_data_age_ms = env_u64("QUINCE_MAX_MARKET_DATA_AGE_MS", 5_000)?;
    validate_risk_limits(max_pos, max_dd, max_freq, max_loss, max_market_data_age_ms)?;
    if live_enabled
        && config.exchange == ExchangeKind::Binance
        && (std::env::var("BINANCE_API_KEY").is_err()
            || std::env::var("BINANCE_SECRET_KEY").is_err())
    {
        return Err(EngineError::Strategy(
            "QUINCE_LIVE=1 requires BINANCE_API_KEY and BINANCE_SECRET_KEY".into(),
        ));
    }

    println!(
        "{}",
        serde_json::json!({
            "status": "ok",
            "offline": true,
            "strategy": strategy,
            "symbol": symbol,
            "exchange": format!("{:?}", config.exchange).to_ascii_lowercase(),
            "network": format!("{:?}", config.network).to_ascii_lowercase(),
            "input_mode": if is_mock { "mock" } else if is_public { "public" } else { "authenticated" },
            "execution_mode": if shadow_enabled { "shadow" } else if live_enabled { "live" } else { "guarded" },
            "risk": { "max_position": max_pos, "max_drawdown": max_dd, "max_order_frequency": max_freq, "max_daily_loss": max_loss, "max_market_data_age_ms": max_market_data_age_ms },
        })
    );
    Ok(())
}

fn validate_runtime_modes(
    is_mock: bool,
    is_public: bool,
    live_enabled: bool,
) -> Result<(), EngineError> {
    if is_mock && is_public {
        return Err(EngineError::Strategy(
            "QUINCE_MOCK and QUINCE_PUBLIC cannot both be enabled".into(),
        ));
    }
    if live_enabled && (is_mock || is_public) {
        return Err(EngineError::Strategy(
            "QUINCE_LIVE cannot be combined with QUINCE_MOCK or QUINCE_PUBLIC".into(),
        ));
    }
    Ok(())
}

fn validate_risk_limits(
    max_pos: f64,
    max_dd: f64,
    max_freq: u32,
    max_loss: f64,
    max_market_data_age_ms: u64,
) -> Result<(), EngineError> {
    if max_pos <= 0.0
        || !(0.0..1.0).contains(&max_dd)
        || max_freq == 0
        || max_loss <= 0.0
        || max_market_data_age_ms == 0
    {
        return Err(EngineError::Strategy(
            "risk limits and QUINCE_MAX_MARKET_DATA_AGE_MS must be positive, and QUINCE_MAX_DRAWDOWN must be in [0, 1)".into(),
        ));
    }
    Ok(())
}

fn env_flag(name: &str) -> Result<bool, EngineError> {
    match std::env::var(name) {
        Ok(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" => Ok(true),
            "0" | "false" | "no" | "" => Ok(false),
            _ => Err(EngineError::Strategy(format!(
                "{name} must be one of 1, true, yes, 0, false, or no"
            ))),
        },
        Err(_) => Ok(false),
    }
}

fn env_f64(name: &str, default: f64) -> Result<f64, EngineError> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| EngineError::Strategy(format!("{name} must be a finite number"))),
        Err(_) => Ok(default),
    }
}

fn env_u32(name: &str, default: u32) -> Result<u32, EngineError> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<u32>()
            .map_err(|_| EngineError::Strategy(format!("{name} must be an unsigned integer"))),
        Err(_) => Ok(default),
    }
}

fn env_u64(name: &str, default: u64) -> Result<u64, EngineError> {
    match std::env::var(name) {
        Ok(value) => value
            .parse::<u64>()
            .map_err(|_| EngineError::Strategy(format!("{name} must be an unsigned integer"))),
        Err(_) => Ok(default),
    }
}

/// Shadow mode is an execution circuit breaker: the VM continues to evaluate
/// live or simulated data, while the engine suppresses every order dispatch.
fn apply_shadow_mode<E: Exchange>(
    engine: &mut Engine<E>,
    enabled: bool,
) -> Result<(), EngineError> {
    if enabled {
        engine.set_deployment_mode(quince::engine::DeploymentMode::Shadow)?;
        tracing::warn!(
            "QUINCE_SHADOW=1: strategy orders are suppressed before journal/exchange dispatch"
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), EngineError> {
    if run_operator_command()? {
        return Ok(());
    }
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .init();

    #[cfg(feature = "profiling")]
    {
        puffin::set_scopes_on(true);
        let addr = "127.0.0.1:29012";
        let server = puffin_http::Server::new(addr).unwrap();
        tracing::info!("puffin profiler listening on http://{addr}");
        std::mem::forget(server);
    }

    let is_mock = env_flag("QUINCE_MOCK")?;
    let is_public = env_flag("QUINCE_PUBLIC")?;
    let wallet_setup_requested = env_flag("QUINCE_WALLET_SETUP")?;
    let skip_wallet_setup = env_flag("QUINCE_SKIP_WALLET_SETUP")?;
    let live_enabled = env_flag("QUINCE_LIVE")?;
    let shadow_enabled = env_flag("QUINCE_SHADOW")?;
    validate_runtime_modes(is_mock, is_public, live_enabled)?;
    let symbol = std::env::var("QUINCE_SYMBOL").unwrap_or_else(|_| "btcusdt".into());
    let strategy =
        std::env::var("QUINCE_STRATEGY").unwrap_or_else(|_| "strategies/test_all.qfl".into());
    let log_path = std::env::var("QUINCE_LOG").unwrap_or_else(|_| "trades.log".into());
    start_dashboard_if_enabled(&log_path)?;
    let mut strategy_config = load_strategy_config(&strategy).map_err(EngineError::Strategy)?;
    if let Ok(exchange) = std::env::var("QUINCE_EXCHANGE") {
        strategy_config.exchange = ExchangeKind::parse(&exchange).map_err(EngineError::Strategy)?;
    }
    if let Ok(network) = std::env::var("QUINCE_NETWORK") {
        strategy_config.network = Network::parse(&network).map_err(EngineError::Strategy)?;
    }

    let max_pos = env_f64("QUINCE_MAX_POSITION", 1.0)?;
    let max_dd = env_f64("QUINCE_MAX_DRAWDOWN", 0.05)?;
    let max_freq = env_u32("QUINCE_MAX_ORDER_FREQ", 10)?;
    let max_loss = env_f64("QUINCE_MAX_DAILY_LOSS", 1000.0)?;
    let max_market_data_age_ms = env_u64("QUINCE_MAX_MARKET_DATA_AGE_MS", 5_000)?;
    validate_risk_limits(max_pos, max_dd, max_freq, max_loss, max_market_data_age_ms)?;

    let risk_config = RiskConfig {
        max_position_size: max_pos,
        max_drawdown: max_dd,
        max_order_freq: max_freq,
        max_daily_loss: max_loss,
        cooldown_after_loss_secs: 60,
    };
    let mut risk = RiskControls::new(risk_config);
    risk.set_max_market_data_age(Duration::from_millis(max_market_data_age_ms));

    if wallet_setup_requested {
        let wallet = wallet::run_setup_wizard().map_err(EngineError::Strategy)?;
        println!("Hyperliquid wallet ready: {}", wallet.hyperliquid_address);
        return Ok(());
    }

    // First interactive launch creates the app-owned EVM wallet before any
    // exchange connection. CI and server deployments stay non-blocking.
    if !skip_wallet_setup
        && wallet::is_interactive()
        && wallet::needs_setup().map_err(EngineError::Strategy)?
    {
        let wallet = wallet::run_setup_wizard().map_err(EngineError::Strategy)?;
        println!("Hyperliquid wallet ready: {}", wallet.hyperliquid_address);
    }

    if is_public {
        match strategy_config.exchange {
            ExchangeKind::Binance => {
                tracing::info!("starting in PUBLIC mode (Binance WS, no API keys)");
                let exchange = quince::exchange::binance::public::BinancePublic::new();
                let mut engine = Engine::new(exchange, &[symbol], &strategy, risk, &log_path)?;
                apply_shadow_mode(&mut engine, shadow_enabled)?;
                engine.run().await
            }
            ExchangeKind::Hyperliquid => {
                tracing::info!(
                    "starting in PUBLIC mode (Hyperliquid {:?})",
                    strategy_config.network
                );
                let exchange = quince::exchange::hyperliquid::public::HyperliquidPublic::new(
                    strategy_config.network == Network::Testnet,
                );
                let mut engine = Engine::new(exchange, &[symbol], &strategy, risk, &log_path)?;
                apply_shadow_mode(&mut engine, shadow_enabled)?;
                engine.run().await
            }
        }
    } else if is_mock {
        tracing::info!("starting in MOCK mode (simulated data)");
        let exchange = mock::MockExchange::new();
        let mut engine = Engine::new(exchange, &[symbol], &strategy, risk, &log_path)?;
        apply_shadow_mode(&mut engine, shadow_enabled)?;
        engine.run().await
    } else if strategy_config.exchange == ExchangeKind::Hyperliquid {
        let wallet = wallet::load_profile()
            .map_err(EngineError::Strategy)?
            .ok_or_else(|| {
                EngineError::Strategy(
                    "Hyperliquid requires a configured wallet; run QUINCE_WALLET_SETUP=1 cargo run first"
                        .into(),
                )
            })?;
        if !wallet::has_private_key().map_err(EngineError::Strategy)? {
            return Err(EngineError::Strategy(
                "Hyperliquid wallet secret is missing from the system keychain; run QUINCE_WALLET_SETUP=1 cargo run"
                    .into(),
            ));
        }
        // Construct the authenticated boundary now, even though mutations are
        // still gated in the adapter. This validates that the keychain secret
        // belongs to the configured address before any future live session.
        let network = if strategy_config.network == Network::Testnet {
            quince::exchange::hyperliquid::execution::HyperliquidNetwork::Testnet
        } else {
            quince::exchange::hyperliquid::execution::HyperliquidNetwork::Mainnet
        };
        let _execution = quince::exchange::hyperliquid::execution::HyperliquidExecution::new(
            network,
            &wallet.hyperliquid_address,
            std::sync::Arc::new(wallet::load_hyperliquid_signer().map_err(EngineError::Strategy)?),
        )
        .map_err(|error| EngineError::Strategy(error.to_string()))?;
        Err(EngineError::Strategy(format!(
            "wallet {} is configured, but Hyperliquid live trading is not enabled yet; use QUINCE_PUBLIC=1 for market data",
            wallet.hyperliquid_address
        )))
    } else if let (Ok(api_key), Ok(secret_key)) = (
        std::env::var("BINANCE_API_KEY"),
        std::env::var("BINANCE_SECRET_KEY"),
    ) {
        let testnet = strategy_config.network == Network::Testnet || env_flag("QUINCE_TESTNET")?;
        if !testnet && !live_enabled {
            return Err(EngineError::Strategy(
                "refusing Binance mainnet trading: set QUINCE_LIVE=1 explicitly".into(),
            ));
        }
        let exchange = quince::exchange::binance::Binance::new(&api_key, &secret_key, testnet);
        let mut engine = Engine::new(exchange, &[symbol], &strategy, risk, &log_path)?;
        apply_shadow_mode(&mut engine, shadow_enabled)?;
        tracing::info!("quince engine starting");
        engine.run().await
    } else {
        if live_enabled {
            return Err(EngineError::Strategy(
                "QUINCE_LIVE=1 requires Binance credentials, or use QUINCE_PUBLIC=1 for market data"
                    .into(),
            ));
        }
        tracing::info!(
            "no BINANCE_API_KEY set — falling back to PUBLIC mode (Binance WS, no keys)"
        );
        let exchange = quince::exchange::binance::public::BinancePublic::new();
        let mut engine = Engine::new(exchange, &[symbol], &strategy, risk, &log_path)?;
        apply_shadow_mode(&mut engine, shadow_enabled)?;
        engine.run().await
    }
}
