mod mock;

use quince::engine::{Engine, EngineError};
use quince::risk::{RiskConfig, RiskControls};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), EngineError> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let is_mock = std::env::var("QUINCE_MOCK").is_ok();
    let symbol = std::env::var("QUINCE_SYMBOL").unwrap_or_else(|_| "btcusdt".into());
    let strategy = std::env::var("QUINCE_STRATEGY")
        .unwrap_or_else(|_| "strategies/test_all.lua".into());
    let log_path = std::env::var("QUINCE_LOG").unwrap_or_else(|_| "trades.log".into());

    let max_pos: f64 = std::env::var("QUINCE_MAX_POSITION")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let max_dd: f64 = std::env::var("QUINCE_MAX_DRAWDOWN")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(0.05);
    let max_freq: u32 = std::env::var("QUINCE_MAX_ORDER_FREQ")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(10);
    let max_loss: f64 = std::env::var("QUINCE_MAX_DAILY_LOSS")
        .ok().and_then(|v| v.parse().ok()).unwrap_or(1000.0);

    let risk_config = RiskConfig {
        max_position_size: max_pos,
        max_drawdown: max_dd,
        max_order_freq: max_freq,
        max_daily_loss: max_loss,
        cooldown_after_loss_secs: 60,
    };
    let risk = RiskControls::new(risk_config);

    if is_mock {
        tracing::info!("starting in MOCK mode (no API keys)");
        let exchange = mock::MockExchange::new();
        let mut engine = Engine::new(exchange, &[symbol], &strategy, risk, &log_path)?;
        engine.run().await
    } else {
        let api_key = std::env::var("BINANCE_API_KEY").expect("BINANCE_API_KEY required");
        let secret_key = std::env::var("BINANCE_SECRET_KEY").expect("BINANCE_SECRET_KEY required");
        let testnet = std::env::var("QUINCE_TESTNET").is_ok();
        let exchange = quince::exchange::binance::Binance::new(&api_key, &secret_key, testnet);
        let mut engine = Engine::new(exchange, &[symbol], &strategy, risk, &log_path)?;
        tracing::info!("quince engine starting");
        engine.run().await
    }
}
