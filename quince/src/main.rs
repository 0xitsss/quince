// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Quince trading bot binary entry point.
//! Configures the trading environment from environment variables, selects
//! mock/public/live exchange mode, and launches the main engine event loop.

mod dashboard;
mod mock;
mod wallet;

use quince::engine::{Engine, EngineError, OrderJournal};
use quince::qfl::config::{load_strategy_config, ExchangeKind, Network};
use quince::risk::{RiskConfig, RiskControls};
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
    let [group, action, path] = args.as_slice() else {
        return Err(EngineError::Strategy(
            "usage: quince journal <inspect|verify> <orders.jsonl>".into(),
        ));
    };
    if group != "journal" || !matches!(action.as_str(), "inspect" | "verify") {
        return Err(EngineError::Strategy(
            "usage: quince journal <inspect|verify> <orders.jsonl>".into(),
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
    if max_pos <= 0.0 || !(0.0..1.0).contains(&max_dd) || max_freq == 0 || max_loss <= 0.0 {
        return Err(EngineError::Strategy(
            "risk limits must be positive, and QUINCE_MAX_DRAWDOWN must be in [0, 1)".into(),
        ));
    }

    let risk_config = RiskConfig {
        max_position_size: max_pos,
        max_drawdown: max_dd,
        max_order_freq: max_freq,
        max_daily_loss: max_loss,
        cooldown_after_loss_secs: 60,
    };
    let risk = RiskControls::new(risk_config);

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
                engine.run().await
            }
        }
    } else if is_mock {
        tracing::info!("starting in MOCK mode (simulated data)");
        let exchange = mock::MockExchange::new();
        let mut engine = Engine::new(exchange, &[symbol], &strategy, risk, &log_path)?;
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
        Err(EngineError::Strategy(
            format!(
                "wallet {} is configured, but Hyperliquid live trading is not enabled yet; use QUINCE_PUBLIC=1 for market data",
                wallet.hyperliquid_address
            ),
        ))
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
        engine.run().await
    }
}
