// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Safe boundary for authenticated Hyperliquid execution.
//!
//! This module deliberately does **not** serialize or submit L1 actions yet.
//! Hyperliquid's action signatures depend on canonical msgpack encoding and a
//! protocol-specific EIP-712 payload.  A locally-valid ECDSA signature is not
//! sufficient proof that the exchange will recover the intended signer.  Until
//! that encoding is covered by official test vectors, every mutating operation
//! fails closed.
//!
//! The types here are still useful now: they keep private-key ownership out of
//! the exchange adapter, bind a signer to an account, validate order intents,
//! and provide one place to add a reviewed signing implementation later.

use super::public::HyperliquidPublic;
use crate::r#trait::{Exchange, ExchangeError, OrderRequest, OrderStatus, Result, Stream};
use quince_core::types::{AccountInfo, Order, OrderType};
use std::sync::Arc;

const MAINNET_EXCHANGE_URL: &str = "https://api.hyperliquid.xyz/exchange";
const TESTNET_EXCHANGE_URL: &str = "https://api.hyperliquid-testnet.xyz/exchange";

/// Hyperliquid deployment selected for an authenticated session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HyperliquidNetwork {
    Mainnet,
    Testnet,
}

impl HyperliquidNetwork {
    pub const fn exchange_url(self) -> &'static str {
        match self {
            Self::Mainnet => MAINNET_EXCHANGE_URL,
            Self::Testnet => TESTNET_EXCHANGE_URL,
        }
    }

    const fn is_testnet(self) -> bool {
        matches!(self, Self::Testnet)
    }
}

/// A signature produced by an external EIP-712/L1-action signer.
///
/// The adapter never receives a private key.  The signer may be backed by an
/// OS keychain, hardware wallet, or a separate signing process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperliquidSignature {
    pub r: String,
    pub s: String,
    pub v: u8,
}

/// Boundary for a future, protocol-reviewed Hyperliquid L1 action signer.
///
/// `action_hash` must be produced by a canonical encoder with official test
/// vectors.  This crate intentionally does not manufacture it yet.
pub trait HyperliquidSigner: Send + Sync {
    /// EVM address controlled by this signer, as a lower-case `0x` address.
    fn address(&self) -> &str;

    /// Sign a canonical Hyperliquid L1-action digest.
    ///
    /// This is not called by the current adapter: no unreviewed encoder is
    /// allowed to reach a live exchange endpoint.
    fn sign_l1_action(
        &self,
        action_hash: [u8; 32],
        network: HyperliquidNetwork,
    ) -> Result<HyperliquidSignature>;
}

/// A checked order intent.  It is intentionally not a wire request.
#[derive(Debug, Clone)]
pub struct ValidatedOrder {
    pub order: Order,
    pub network: HyperliquidNetwork,
    pub account_address: String,
}

/// Authenticated adapter shell.
///
/// Public-data methods work through [`HyperliquidPublic`]. Mutating methods
/// reject until canonical action encoding, signing vectors, submission, and
/// reconciliation are all implemented together.
pub struct HyperliquidExecution {
    network: HyperliquidNetwork,
    account_address: String,
    signer: Arc<dyn HyperliquidSigner>,
    public: HyperliquidPublic,
}

impl HyperliquidExecution {
    /// Creates an execution boundary only when the signer belongs to the
    /// configured account.  Vault/subaccount delegation is intentionally not
    /// accepted yet, because its authorization and reconciliation semantics
    /// need separate implementation.
    pub fn new(
        network: HyperliquidNetwork,
        account_address: impl AsRef<str>,
        signer: Arc<dyn HyperliquidSigner>,
    ) -> Result<Self> {
        let account_address = normalize_evm_address(account_address.as_ref())?;
        let signer_address = normalize_evm_address(signer.address())?;
        if signer_address != account_address {
            return Err(ExchangeError::Auth(
                "Hyperliquid signer address must match the configured account; vault and subaccount delegation are not enabled".into(),
            ));
        }

        Ok(Self {
            network,
            account_address,
            signer,
            public: HyperliquidPublic::new(network.is_testnet()),
        })
    }

    pub const fn network(&self) -> HyperliquidNetwork {
        self.network
    }

    pub fn account_address(&self) -> &str {
        &self.account_address
    }

    pub const fn exchange_url(&self) -> &'static str {
        self.network.exchange_url()
    }

    /// Validates an intent before it can enter a future signing queue.
    ///
    /// Validation is purposefully strict: unsupported stop-loss/take-profit
    /// fields and non-finite numeric values cannot be silently translated into
    /// a different exchange action.
    pub fn validate_order(&self, order: Order) -> Result<ValidatedOrder> {
        validate_order(&order)?;
        Ok(ValidatedOrder {
            order,
            network: self.network,
            account_address: self.account_address.clone(),
        })
    }

    fn execution_disabled() -> ExchangeError {
        ExchangeError::Order(
            "Hyperliquid live execution is disabled: canonical L1 action encoding, official signing test vectors, submission, and reconciliation must be implemented before orders can be sent".into(),
        )
    }
}

fn validate_order(order: &Order) -> Result<()> {
    let symbol = order.symbol.trim();
    if symbol.is_empty() || symbol.len() > 64 || !symbol.bytes().all(|b| b.is_ascii_alphanumeric())
    {
        return Err(ExchangeError::Order(
            "Hyperliquid symbol must be a non-empty ASCII alphanumeric coin name".into(),
        ));
    }
    if !order.qty.is_finite() || order.qty <= 0.0 {
        return Err(ExchangeError::Order(
            "Hyperliquid order quantity must be finite and greater than zero".into(),
        ));
    }
    match (order.order_type, order.price) {
        (OrderType::Limit, Some(price)) if price.is_finite() && price > 0.0 => {}
        (OrderType::Limit, _) => {
            return Err(ExchangeError::Order(
                "Hyperliquid limit orders require a finite positive price".into(),
            ));
        }
        (OrderType::Market, Some(price)) if !price.is_finite() || price <= 0.0 => {
            return Err(ExchangeError::Order(
                "Hyperliquid market-order price, when supplied, must be finite and positive".into(),
            ));
        }
        (OrderType::Market, _) => {}
    }
    if order.stop_loss.is_some() || order.take_profit.is_some() {
        return Err(ExchangeError::Order(
            "Hyperliquid trigger orders are not mapped by this execution boundary".into(),
        ));
    }
    Ok(())
}

fn normalize_evm_address(address: &str) -> Result<String> {
    let address = address.trim();
    if address.len() != 42
        || !address.starts_with("0x")
        || !address[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ExchangeError::Auth(
            "Hyperliquid account and signer addresses must be 20-byte 0x-prefixed hexadecimal EVM addresses".into(),
        ));
    }
    Ok(address.to_ascii_lowercase())
}

#[async_trait::async_trait]
impl Exchange for HyperliquidExecution {
    async fn subscribe(&self, symbols: &[String]) -> Result<Stream> {
        self.public.subscribe(symbols).await
    }

    async fn place_order(&self, request: OrderRequest) -> Result<String> {
        // Preserve validation even while disabled, so callers receive a useful
        // local error for malformed intent without any network side effect.
        self.validate_order(request.order)?;
        let _ = &self.signer;
        Err(Self::execution_disabled())
    }

    async fn cancel_order(&self, _symbol: &str, _order_id: &str) -> Result<()> {
        Err(Self::execution_disabled())
    }

    async fn order_status(&self, _symbol: &str, _order_id: &str) -> Result<OrderStatus> {
        Err(ExchangeError::Order(
            "Hyperliquid authenticated order reconciliation is not implemented".into(),
        ))
    }

    async fn account_info(&self) -> Result<AccountInfo> {
        self.public.account_info_for(&self.account_address).await
    }

    async fn current_price(&self, symbol: &str) -> Result<f64> {
        self.public.current_price(symbol).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quince_core::types::{OrderType, Side};
    use std::sync::Arc;

    const ADDRESS: &str = "0x0123456789abcdef0123456789abcdef01234567";

    struct TestSigner {
        address: String,
    }

    impl HyperliquidSigner for TestSigner {
        fn address(&self) -> &str {
            &self.address
        }

        fn sign_l1_action(
            &self,
            _action_hash: [u8; 32],
            _network: HyperliquidNetwork,
        ) -> Result<HyperliquidSignature> {
            panic!("the disabled execution path must never request a signature")
        }
    }

    fn adapter(network: HyperliquidNetwork) -> HyperliquidExecution {
        HyperliquidExecution::new(
            network,
            format!("0x{}", ADDRESS[2..].to_uppercase()),
            Arc::new(TestSigner {
                address: ADDRESS.to_string(),
            }),
        )
        .unwrap()
    }

    fn limit_order() -> Order {
        Order {
            symbol: "BTC".into(),
            side: Side::Buy,
            qty: 0.1,
            price: Some(100_000.0),
            order_type: OrderType::Limit,
            reduce_only: false,
            stop_loss: None,
            take_profit: None,
        }
    }

    #[test]
    fn mainnet_and_testnet_use_the_official_exchange_endpoints() {
        assert_eq!(
            adapter(HyperliquidNetwork::Mainnet).exchange_url(),
            MAINNET_EXCHANGE_URL
        );
        assert_eq!(
            adapter(HyperliquidNetwork::Testnet).exchange_url(),
            TESTNET_EXCHANGE_URL
        );
    }

    #[test]
    fn signer_must_match_the_configured_account() {
        let result = HyperliquidExecution::new(
            HyperliquidNetwork::Testnet,
            ADDRESS,
            Arc::new(TestSigner {
                address: "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            }),
        );
        assert!(matches!(result, Err(ExchangeError::Auth(_))));
    }

    #[test]
    fn addresses_are_normalized_and_checked() {
        let execution = adapter(HyperliquidNetwork::Testnet);
        assert_eq!(execution.account_address(), ADDRESS);
        assert!(normalize_evm_address("0xnot-an-address").is_err());
    }

    #[test]
    fn rejects_invalid_and_unsupported_order_intents() {
        let execution = adapter(HyperliquidNetwork::Testnet);
        let mut order = limit_order();
        order.qty = f64::NAN;
        assert!(execution.validate_order(order).is_err());

        let mut order = limit_order();
        order.price = None;
        assert!(execution.validate_order(order).is_err());

        let mut order = limit_order();
        order.stop_loss = Some(90_000.0);
        assert!(execution.validate_order(order).is_err());
    }

    #[tokio::test]
    async fn live_order_path_fails_closed_without_signing_or_network_io() {
        let execution = adapter(HyperliquidNetwork::Testnet);
        let error = execution
            .place_order(OrderRequest {
                client_order_id: "test-order".into(),
                order: limit_order(),
            })
            .await
            .unwrap_err();
        assert!(matches!(error, ExchangeError::Order(message) if message.contains("disabled")));
    }
}
