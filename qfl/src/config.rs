// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Strategy-level exchange configuration parsed from QFL directives.

use crate::ast::Stmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExchangeKind {
    #[default]
    Binance,
    Hyperliquid,
}

impl ExchangeKind {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "binance" => Ok(Self::Binance),
            "hyperliquid" | "hl" => Ok(Self::Hyperliquid),
            _ => Err(format!(
                "unsupported exchange '{value}'; expected binance or hyperliquid"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Network {
    #[default]
    Mainnet,
    Testnet,
}

impl Network {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_ascii_lowercase().as_str() {
            "mainnet" => Ok(Self::Mainnet),
            "testnet" => Ok(Self::Testnet),
            _ => Err(format!(
                "unsupported network '{value}'; expected mainnet or testnet"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StrategyConfig {
    pub exchange: ExchangeKind,
    pub network: Network,
}

/// Parse configuration directives without compiling the strategy bytecode.
pub fn parse_strategy_config(source: &str) -> Result<StrategyConfig, String> {
    let program = crate::parser::parse(source).map_err(|e| e.to_string())?;
    let mut config = StrategyConfig::default();

    for stmt in program {
        match stmt {
            Stmt::Exchange { name } => config.exchange = ExchangeKind::parse(&name)?,
            Stmt::Network { name } => config.network = Network::parse(&name)?,
            _ => {}
        }
    }
    Ok(config)
}

pub fn load_strategy_config(path: &str) -> Result<StrategyConfig, String> {
    let source = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    parse_strategy_config(&source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_binance_mainnet() {
        assert_eq!(
            parse_strategy_config("function on_eval() end").unwrap(),
            StrategyConfig::default()
        );
    }

    #[test]
    fn parses_hyperliquid_testnet_directives() {
        let config = parse_strategy_config(
            "@exchange hyperliquid\n@network testnet\nfunction on_eval() end",
        )
        .unwrap();
        assert_eq!(config.exchange, ExchangeKind::Hyperliquid);
        assert_eq!(config.network, Network::Testnet);
    }

    #[test]
    fn rejects_unknown_exchange() {
        assert!(parse_strategy_config("@exchange kraken").is_err());
    }
}
