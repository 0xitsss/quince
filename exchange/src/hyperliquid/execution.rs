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

use super::user_data;
use super::{public::HyperliquidPublic, signing};
use crate::r#trait::{
    Exchange, ExchangeError, OrderRequest, OrderStatus, Result, Stream, StreamMsg,
};
use chrono::{DateTime, Duration, Utc};
use quince_core::types::{AccountInfo, Order, OrderType, Side};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};

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

    pub const fn info_url(self) -> &'static str {
        match self {
            Self::Mainnet => "https://api.hyperliquid.xyz/info",
            Self::Testnet => "https://api.hyperliquid-testnet.xyz/info",
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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

/// Authoritative perp asset metadata used to bind a user coin to its protocol
/// asset index and permitted size precision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperliquidPerpMeta {
    assets: HashMap<String, PerpAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpAsset {
    pub index: u32,
    pub size_decimals: u8,
}

/// Fully prepared, signed exchange payload. It is intentionally separate from
/// transport so callers can journal the immutable idempotency context before
/// any network side effect occurs.
#[derive(Debug, Clone)]
pub struct PreparedHyperliquidOrder {
    pub client_order_id: String,
    pub nonce: u64,
    pub payload: serde_json::Value,
}

/// Inputs captured immediately before signing. An absent or stale market view
/// is a hard execution failure, never a reason to reuse a last known quote.
#[derive(Debug, Clone)]
pub struct ExecutionPreflight {
    pub market_observed_at: DateTime<Utc>,
    pub max_market_age: Duration,
}

impl ExecutionPreflight {
    pub fn check(&self, now: DateTime<Utc>) -> Result<()> {
        if self.max_market_age <= Duration::zero() {
            return Err(ExchangeError::Order("invalid market-data TTL".into()));
        }
        let age = now.signed_duration_since(self.market_observed_at);
        if age < Duration::zero() || age > self.max_market_age {
            return Err(ExchangeError::Order(
                "Hyperliquid execution blocked: market data is stale or clock-skewed".into(),
            ));
        }
        Ok(())
    }
}

impl HyperliquidPerpMeta {
    pub fn parse(value: &serde_json::Value) -> Result<Self> {
        let universe = value
            .get("universe")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| ExchangeError::Rest("missing Hyperliquid meta universe".into()))?;
        let mut assets = HashMap::with_capacity(universe.len());
        for (index, asset) in universe.iter().enumerate() {
            let name = asset
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| {
                    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_alphanumeric())
                })
                .ok_or_else(|| ExchangeError::Rest("invalid Hyperliquid meta asset name".into()))?;
            let size_decimals = asset
                .get("szDecimals")
                .and_then(serde_json::Value::as_u64)
                .filter(|value| *value <= 8)
                .ok_or_else(|| ExchangeError::Rest("invalid Hyperliquid meta szDecimals".into()))?
                as u8;
            if assets
                .insert(
                    name.to_ascii_uppercase(),
                    PerpAsset {
                        index: index as u32,
                        size_decimals,
                    },
                )
                .is_some()
            {
                return Err(ExchangeError::Rest(
                    "duplicate Hyperliquid meta asset".into(),
                ));
            }
        }
        Ok(Self { assets })
    }

    pub fn asset(&self, coin: &str) -> Result<&PerpAsset> {
        self.assets
            .get(&coin.to_ascii_uppercase())
            .ok_or_else(|| ExchangeError::Order(format!("unknown Hyperliquid perp asset {coin}")))
    }

    /// Formats a size only when it is exactly representable at the exchange's
    /// declared precision. Silent client-side rounding changes risk exposure.
    pub fn format_size(&self, coin: &str, size: f64) -> Result<String> {
        if !size.is_finite() || size <= 0.0 {
            return Err(ExchangeError::Order(
                "invalid Hyperliquid order size".into(),
            ));
        }
        let decimals = self.asset(coin)?.size_decimals as usize;
        let formatted = format!("{size:.decimals$}");
        let parsed: f64 = formatted
            .parse()
            .map_err(|_| ExchangeError::Order("format Hyperliquid order size".into()))?;
        let tolerance = size.abs().max(1.0) * 1e-12;
        if (parsed - size).abs() > tolerance {
            return Err(ExchangeError::Order(format!(
                "Hyperliquid size {size} exceeds {decimals}-decimal precision"
            )));
        }
        Ok(formatted
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_owned())
    }
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
    nonce: AtomicU64,
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
            nonce: AtomicU64::new(unix_time_ms()),
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

    /// Returns an exchange nonce that is monotonic even when multiple tasks
    /// prepare actions in the same millisecond or the wall clock moves back.
    /// A nonce is allocated only during final action preparation, never retried
    /// after an ambiguous network submission.
    pub fn next_nonce(&self) -> u64 {
        let now = unix_time_ms();
        loop {
            let current = self.nonce.load(Ordering::Relaxed);
            let next = current.max(now).saturating_add(1);
            if self
                .nonce
                .compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return next;
            }
        }
    }

    /// Fetches a fresh, authoritative perp metadata snapshot before action
    /// construction. This is read-only and deliberately separate from order
    /// submission, so stale/invalid metadata cannot become a signed payload.
    pub async fn load_perp_meta(&self) -> Result<HyperliquidPerpMeta> {
        let value: serde_json::Value = reqwest::Client::new()
            .post(self.network.info_url())
            .json(&serde_json::json!({ "type": "meta" }))
            .send()
            .await
            .map_err(|error| ExchangeError::Rest(format!("fetch Hyperliquid meta: {error}")))?
            .error_for_status()
            .map_err(|error| ExchangeError::Rest(format!("Hyperliquid meta status: {error}")))?
            .json()
            .await
            .map_err(|error| ExchangeError::Rest(format!("parse Hyperliquid meta: {error}")))?;
        HyperliquidPerpMeta::parse(&value)
    }

    /// Creates a signed, IOC limit-order payload from a fresh authoritative
    /// metadata snapshot. Submission remains a distinct testnet-only step.
    pub fn prepare_limit_order(
        &self,
        request: OrderRequest,
        meta: &HyperliquidPerpMeta,
        market: &ExecutionPreflight,
    ) -> Result<PreparedHyperliquidOrder> {
        self.check_preflight(&request.order, meta, market)?;
        self.validate_order(request.order.clone())?;
        if request.client_order_id.trim().is_empty() {
            return Err(ExchangeError::Order("missing client order id".into()));
        }
        if request.order.order_type != OrderType::Limit || request.order.reduce_only {
            return Err(ExchangeError::Order(
                "Hyperliquid initial execution supports IOC non-reduce-only limit orders only"
                    .into(),
            ));
        }
        let asset = meta.asset(&request.order.symbol)?;
        let price = format_wire_number(request.order.price.unwrap())?;
        let size = meta.format_size(&request.order.symbol, request.order.qty)?;
        let nonce = self.next_nonce();
        let connection_id = signing::limit_order_connection_id(
            asset.index,
            matches!(request.order.side, Side::Buy),
            &price,
            &size,
            nonce,
        )
        .map_err(ExchangeError::Order)?;
        let signature = self.signer.sign_l1_action(connection_id, self.network)?;
        Ok(PreparedHyperliquidOrder {
            client_order_id: request.client_order_id,
            nonce,
            payload: serde_json::json!({
                "action": {"type": "order", "orders": [{"a": asset.index, "b": matches!(request.order.side, Side::Buy), "p": price, "s": size, "r": false, "t": {"limit": {"tif": "Ioc"}}}], "grouping": "na"},
                "signature": signature,
                "nonce": nonce,
                "vaultAddress": serde_json::Value::Null,
            }),
        })
    }

    /// Validates market freshness, intent and exchange metadata together.
    /// Callers with a market-data timestamp must use this immediately before
    /// creating a signed action.
    pub fn check_preflight(
        &self,
        order: &Order,
        meta: &HyperliquidPerpMeta,
        market: &ExecutionPreflight,
    ) -> Result<()> {
        market.check(Utc::now())?;
        validate_order(order)?;
        meta.asset(&order.symbol)?;
        meta.format_size(&order.symbol, order.qty)?;
        Ok(())
    }

    /// Submits a previously journaled payload to Hyperliquid testnet only.
    /// Mainnet execution stays explicitly disabled until the full order
    /// reconciler owns ambiguous transport outcomes.
    pub async fn submit_testnet_order(
        &self,
        prepared: &PreparedHyperliquidOrder,
    ) -> Result<String> {
        if self.network != HyperliquidNetwork::Testnet {
            return Err(ExchangeError::Order(
                "refusing Hyperliquid mainnet submission; testnet validation is required".into(),
            ));
        }
        let response: serde_json::Value = reqwest::Client::new()
            .post(self.exchange_url())
            .json(&prepared.payload)
            .send()
            .await
            .map_err(|error| {
                ExchangeError::Rest(format!("submit Hyperliquid testnet order: {error}"))
            })?
            .error_for_status()
            .map_err(|error| {
                ExchangeError::Rest(format!("Hyperliquid testnet order status: {error}"))
            })?
            .json()
            .await
            .map_err(|error| {
                ExchangeError::Rest(format!("parse Hyperliquid testnet order: {error}"))
            })?;
        parse_order_id(&response)
    }

    /// Prepares a signed cancel payload from fresh metadata.
    pub fn prepare_cancel(
        &self,
        symbol: &str,
        order_id: &str,
        meta: &HyperliquidPerpMeta,
    ) -> Result<PreparedHyperliquidOrder> {
        let asset = meta.asset(symbol)?;
        let oid = order_id
            .parse::<u64>()
            .ok()
            .filter(|oid| *oid > 0)
            .ok_or_else(|| ExchangeError::Order("invalid Hyperliquid exchange order id".into()))?;
        let nonce = self.next_nonce();
        let connection_id =
            signing::cancel_connection_id(asset.index, oid, nonce).map_err(ExchangeError::Order)?;
        let signature = self.signer.sign_l1_action(connection_id, self.network)?;
        Ok(PreparedHyperliquidOrder {
            client_order_id: format!("cancel:{oid}"),
            nonce,
            payload: serde_json::json!({
                "action": {"type": "cancel", "cancels": [{"a": asset.index, "o": oid}]},
                "signature": signature,
                "nonce": nonce,
                "vaultAddress": serde_json::Value::Null,
            }),
        })
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

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn format_wire_number(value: f64) -> Result<String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(ExchangeError::Order(
            "invalid Hyperliquid wire number".into(),
        ));
    }
    let formatted = format!("{value:.8}");
    let parsed: f64 = formatted
        .parse()
        .map_err(|_| ExchangeError::Order("format Hyperliquid wire number".into()))?;
    if (parsed - value).abs() > value.abs().max(1.0) * 1e-12 {
        return Err(ExchangeError::Order(
            "Hyperliquid price exceeds 8-decimal wire precision".into(),
        ));
    }
    Ok(formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned())
}

fn parse_order_id(response: &serde_json::Value) -> Result<String> {
    if response.get("status").and_then(serde_json::Value::as_str) != Some("ok") {
        return Err(ExchangeError::Order(format!(
            "Hyperliquid rejected order: {}",
            response
                .get("response")
                .and_then(|value| value.get("data"))
                .unwrap_or(response)
        )));
    }
    let status = response
        .pointer("/response/data/statuses/0")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| ExchangeError::Order("missing Hyperliquid order status".into()))?;
    for outcome in ["resting", "filled"] {
        if let Some(oid) = status
            .get(outcome)
            .and_then(|value| value.get("oid"))
            .and_then(|value| value.as_u64())
        {
            return Ok(oid.to_string());
        }
    }
    Err(ExchangeError::Order(
        "Hyperliquid order response was neither resting nor filled".into(),
    ))
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
        let public_stream = self.public.subscribe(symbols).await?;
        let (tx, rx) = crossbeam_channel::bounded(1024);
        let public_tx = tx.clone();
        let integrity_rx = rx.clone();
        tokio::task::spawn_blocking(move || {
            while let Ok(event) = public_stream.rx.recv() {
                if public_tx.try_send(event).is_ok() {
                    continue;
                }
                // A full merged ingress invalidates the ordering/freshness of
                // both feeds. Replace exactly one stale event with a durable
                // reconciliation signal rather than silently dropping data.
                let _ = integrity_rx.try_recv();
                let _ = public_tx.try_send(StreamMsg::ReconcileRequired {
                    source: "hyperliquid-execution",
                    reason: "merged public/private ingress overflow".into(),
                });
            }
        });
        user_data::start_user_data_stream(
            self.account_address.clone(),
            self.network.is_testnet(),
            tx,
        )
        .await?;
        Ok(Stream { rx })
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
            Ok(HyperliquidSignature {
                r: "0x01".into(),
                s: "0x02".into(),
                v: 27,
            })
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

    #[test]
    fn nonces_are_strictly_monotonic() {
        let execution = adapter(HyperliquidNetwork::Testnet);
        let first = execution.next_nonce();
        let second = execution.next_nonce();
        assert!(second > first);
    }

    #[test]
    fn meta_binds_coin_to_its_authoritative_protocol_index() {
        let meta = HyperliquidPerpMeta::parse(&serde_json::json!({
            "universe": [{"name": "BTC", "szDecimals": 5}, {"name": "ETH", "szDecimals": 4}]
        }))
        .unwrap();
        assert_eq!(
            meta.asset("btc").unwrap(),
            &PerpAsset {
                index: 0,
                size_decimals: 5
            }
        );
        assert!(meta.asset("SOL").is_err());
        assert_eq!(meta.format_size("BTC", 0.12345).unwrap(), "0.12345");
        assert!(meta.format_size("BTC", 0.123456).is_err());
    }

    #[test]
    fn prepared_order_binds_meta_nonce_and_signature_before_transport() {
        let execution = adapter(HyperliquidNetwork::Testnet);
        let meta = HyperliquidPerpMeta::parse(&serde_json::json!({
            "universe": [{"name": "BTC", "szDecimals": 5}]
        }))
        .unwrap();
        let prepared = execution
            .prepare_limit_order(
                OrderRequest {
                    client_order_id: "018f1c10-5c11-7000-8000-000000000001".into(),
                    order: limit_order(),
                },
                &meta,
                &ExecutionPreflight {
                    market_observed_at: Utc::now(),
                    max_market_age: Duration::seconds(1),
                },
            )
            .unwrap();
        assert_eq!(prepared.payload["action"]["orders"][0]["a"], 0);
        assert_eq!(prepared.payload["signature"]["v"], 27);
        assert_eq!(prepared.payload["nonce"], prepared.nonce);
    }

    #[test]
    fn prepared_cancel_binds_symbol_to_meta_asset() {
        let execution = adapter(HyperliquidNetwork::Testnet);
        let meta = HyperliquidPerpMeta::parse(&serde_json::json!({
            "universe": [{"name": "BTC", "szDecimals": 5}]
        }))
        .unwrap();
        let prepared = execution.prepare_cancel("BTC", "42", &meta).unwrap();
        assert_eq!(prepared.payload["action"]["cancels"][0]["a"], 0);
        assert_eq!(prepared.payload["action"]["cancels"][0]["o"], 42);
        assert!(execution
            .prepare_cancel("BTC", "not-an-oid", &meta)
            .is_err());
    }

    #[test]
    fn preflight_rejects_stale_and_clock_skewed_market_data() {
        let now = Utc::now();
        assert!(ExecutionPreflight {
            market_observed_at: now - Duration::seconds(2),
            max_market_age: Duration::seconds(1),
        }
        .check(now)
        .is_err());
        assert!(ExecutionPreflight {
            market_observed_at: now + Duration::seconds(1),
            max_market_age: Duration::seconds(1),
        }
        .check(now)
        .is_err());
    }

    #[test]
    fn order_response_requires_a_terminal_exchange_order_id() {
        assert_eq!(
            parse_order_id(&serde_json::json!({
                "status": "ok", "response": {"data": {"statuses": [{"filled": {"oid": 42}}]}}
            }))
            .unwrap(),
            "42"
        );
        assert!(parse_order_id(
            &serde_json::json!({"status": "ok", "response": {"data": {"statuses": [{}]}}})
        )
        .is_err());
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
