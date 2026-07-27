// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Authenticated Binance exchange implementation.
//! Provides REST order placement, account queries, and WebSocket-backed
//! market data streaming via the [`Binance`] struct.

pub mod filters;
pub mod public;
pub mod types;
pub mod user_data;
pub mod ws;

use crate::r#trait::{Exchange, ExchangeError, OrderRequest, OrderStatus, Result, Stream};
use crossbeam_channel;
use quince_core::types::*;
use serde_json::{Map, Value};
use std::sync::OnceLock;
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Binance {
    api_key: String,
    secret_key: String,
    testnet: bool,
    client: OnceLock<ws::WsClient>,
    filters: OnceLock<filters::BinanceFilters>,
}

impl Binance {
    pub fn new(api_key: &str, secret_key: &str, testnet: bool) -> Self {
        Self {
            api_key: api_key.to_string(),
            secret_key: secret_key.to_string(),
            testnet,
            client: OnceLock::new(),
            filters: OnceLock::new(),
        }
    }

    fn validate_credentials(&self) -> Result<()> {
        if self.api_key.trim().is_empty() || self.secret_key.trim().is_empty() {
            return Err(ExchangeError::Auth(
                "Binance API key and secret key must both be configured".into(),
            ));
        }
        Ok(())
    }

    async fn request(&self, method: &str, params: Map<String, Value>) -> Result<Value> {
        let req_tx = self.req_tx()?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        req_tx
            .try_send(ws::WsRequest {
                method: method.into(),
                params,
                response_tx: tx,
            })
            .map_err(|error| match error {
                crossbeam_channel::TrySendError::Full(_) => ExchangeError::Timeout,
                crossbeam_channel::TrySendError::Disconnected(_) => ExchangeError::Disconnected,
            })?;

        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(ExchangeError::Disconnected),
            Err(_) => Err(ExchangeError::Timeout),
        }
    }

    fn exchange_info_url(&self) -> &'static str {
        if self.testnet {
            "https://testnet.binancefuture.com/fapi/v1/exchangeInfo"
        } else {
            "https://fapi.binance.com/fapi/v1/exchangeInfo"
        }
    }

    async fn load_filters(&self) -> Result<&filters::BinanceFilters> {
        if let Some(filters) = self.filters.get() {
            return Ok(filters);
        }
        let response = reqwest::Client::new()
            .get(self.exchange_info_url())
            .send()
            .await
            .map_err(|error| ExchangeError::Rest(format!("fetch Binance exchangeInfo: {error}")))?
            .error_for_status()
            .map_err(|error| {
                ExchangeError::Rest(format!("Binance exchangeInfo status: {error}"))
            })?;
        let value = response
            .json::<Value>()
            .await
            .map_err(|error| ExchangeError::Rest(format!("parse Binance exchangeInfo: {error}")))?;
        let parsed = filters::BinanceFilters::from_exchange_info(&value)?;
        let _ = self.filters.set(parsed);
        self.filters
            .get()
            .ok_or_else(|| ExchangeError::Rest("Binance exchangeInfo cache unavailable".into()))
    }

    async fn symbol_filters(&self, symbol: &str) -> Result<&filters::SymbolFilters> {
        self.load_filters().await?.symbol(symbol)
    }

    fn order_params(
        &self,
        request: &OrderRequest,
        symbol_filters: &filters::SymbolFilters,
    ) -> Result<Map<String, Value>> {
        let order = &request.order;
        let symbol = normalize_symbol(&order.symbol)?;
        let normalized_qty = symbol_filters.normalize_quantity(order.qty)?;

        let mut params = Map::new();
        params.insert("symbol".into(), Value::String(symbol));
        params.insert(
            "side".into(),
            Value::String(match order.side {
                Side::Buy => "BUY".into(),
                Side::Sell => "SELL".into(),
            }),
        );
        params.insert(
            "type".into(),
            Value::String(match order.order_type {
                OrderType::Market => "MARKET".into(),
                OrderType::Limit => "LIMIT".into(),
            }),
        );
        let client_order_id = validate_client_order_id(&request.client_order_id)?;
        params.insert("newClientOrderId".into(), Value::String(client_order_id));
        if order.reduce_only {
            params.insert("reduceOnly".into(), Value::String("true".into()));
        }

        match (order.order_type, order.price) {
            (OrderType::Market, None) => {
                params.insert("quantity".into(), Value::String(normalized_qty.to_string()));
            }
            (OrderType::Market, Some(_)) => {
                return Err(ExchangeError::Order(
                    "market orders must not include a limit price".into(),
                ));
            }
            (OrderType::Limit, Some(price)) if price.is_finite() && price > 0.0 => {
                let normalized = symbol_filters.normalize_limit_order(price, normalized_qty)?;
                params.insert("quantity".into(), Value::String(normalized.qty.to_string()));
                params.insert("price".into(), Value::String(normalized.price.to_string()));
                params.insert("timeInForce".into(), Value::String("GTC".into()));
            }
            (OrderType::Limit, _) => {
                return Err(ExchangeError::Order(
                    "limit orders require a positive finite price".into(),
                ));
            }
        }
        Ok(params)
    }

    fn req_tx(&self) -> Result<crossbeam_channel::Sender<ws::WsRequest>> {
        self.client
            .get()
            .map(|c| c.req_tx.clone())
            .ok_or(ExchangeError::Disconnected)
    }

    fn parse_account_info(result: Value) -> Result<AccountInfo> {
        let assets = result["assets"]
            .as_array()
            .ok_or_else(|| ExchangeError::Rest("account response is missing assets".into()))?;
        let mut balances = Vec::with_capacity(assets.len());
        for asset in assets {
            balances.push(Balance {
                asset: required_text(asset, "asset")?.to_string(),
                wallet: required_f64(asset, "walletBalance")?,
                cross_wallet: required_f64(asset, "crossWalletBalance")?,
            });
        }

        let positions_json = result["positions"]
            .as_array()
            .ok_or_else(|| ExchangeError::Rest("account response is missing positions".into()))?;
        let mut positions = Vec::with_capacity(positions_json.len());
        for position in positions_json {
            let size = required_f64(position, "positionAmt")?;
            let side = if size > 0.0 {
                PositionSide::Long
            } else if size < 0.0 {
                PositionSide::Short
            } else {
                PositionSide::None
            };
            positions.push(Position {
                symbol: required_text(position, "symbol")?.to_string(),
                side,
                size: size.abs(),
                entry_price: required_f64(position, "entryPrice")?,
                unrealized_pnl: required_f64(position, "unrealizedProfit")?,
            });
        }

        Ok(AccountInfo {
            balances,
            positions,
        })
    }
}

fn required_text<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value[field]
        .as_str()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| ExchangeError::Rest(format!("account response is missing {field}")))
}

fn required_f64(value: &Value, field: &str) -> Result<f64> {
    let parsed = required_text(value, field)?
        .parse::<f64>()
        .map_err(|_| ExchangeError::Rest(format!("account response has invalid {field}")))?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(ExchangeError::Rest(format!(
            "account response has non-finite {field}"
        )))
    }
}

fn normalize_symbol(symbol: &str) -> Result<String> {
    let normalized = symbol.trim().to_ascii_uppercase();
    if !(3..=20).contains(&normalized.len())
        || !normalized.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(ExchangeError::Order(
            "symbol must contain 3-20 ASCII alphanumeric characters".into(),
        ));
    }
    Ok(normalized)
}

#[async_trait::async_trait]
impl Exchange for Binance {
    async fn subscribe(&self, symbols: &[String]) -> Result<Stream> {
        self.validate_credentials()?;
        if symbols.is_empty() {
            return Err(ExchangeError::Ws("at least one symbol is required".into()));
        }
        for symbol in symbols {
            self.symbol_filters(symbol).await?;
        }
        if self.client.get().is_some() {
            return Err(ExchangeError::Ws(
                "Binance adapter is already subscribed; create a new adapter after disconnect"
                    .into(),
            ));
        }
        let ws = ws::BinanceWs::new(&self.api_key, &self.secret_key, self.testnet);
        let (client, rx) = ws.connect(symbols).await?;
        user_data::start_user_data_stream(
            self.api_key.clone(),
            self.testnet,
            client.stream_tx.clone(),
        )
        .await?;
        let _ = self.client.set(client);
        Ok(Stream { rx })
    }

    async fn place_order(&self, request: OrderRequest) -> Result<String> {
        let client_order_id = request.client_order_id.clone();
        let symbol_filters = self.symbol_filters(&request.order.symbol).await?;
        let params = self.order_params(&request, symbol_filters)?;
        let result = self
            .request("order.place", params)
            .await
            .map_err(|error| match error {
                // An order could have reached Binance even if its response was
                // lost. The client ID is the reconciliation handle; never retry
                // blindly after this error.
                ExchangeError::Timeout | ExchangeError::Disconnected => ExchangeError::Order(
                    format!(
                        "order outcome is unknown; reconcile newClientOrderId={client_order_id} before retrying"
                    ),
                ),
                other => other,
            })?;

        result["orderId"]
            .as_u64()
            .map(|id| id.to_string())
            .ok_or_else(|| ExchangeError::Order("missing orderId".into()))
    }

    async fn cancel_order(&self, symbol: &str, order_id: &str) -> Result<()> {
        let mut params = Map::new();
        params.insert("symbol".into(), Value::String(normalize_symbol(symbol)?));
        params.insert(
            "orderId".into(),
            Value::String(validate_order_id(order_id)?),
        );
        self.request("order.cancel", params).await?;
        Ok(())
    }

    async fn order_status(&self, symbol: &str, order_id: &str) -> Result<OrderStatus> {
        let mut params = Map::new();
        params.insert("symbol".into(), Value::String(normalize_symbol(symbol)?));
        params.insert(
            "orderId".into(),
            Value::String(validate_order_id(order_id)?),
        );
        let result = self.request("order.status", params).await?;

        Ok(OrderStatus {
            order_id: result["orderId"]
                .as_u64()
                .map(|v| v.to_string())
                .unwrap_or_default(),
            symbol: result["symbol"].as_str().unwrap_or("").to_string(),
            side: Side::from_taker(result["side"].as_str().unwrap_or("")),
            qty: result["origQty"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0.0),
            filled_qty: result["executedQty"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0.0),
            price: result["price"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0.0),
            avg_price: result["avgPrice"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .unwrap_or(0.0),
            status: result["status"].as_str().unwrap_or("").to_string(),
        })
    }

    async fn order_status_by_client_id(
        &self,
        symbol: &str,
        client_order_id: &str,
    ) -> Result<OrderStatus> {
        let mut params = Map::new();
        params.insert("symbol".into(), Value::String(normalize_symbol(symbol)?));
        params.insert(
            "origClientOrderId".into(),
            Value::String(validate_client_order_id(client_order_id)?),
        );
        let result = self.request("order.status", params).await?;
        Ok(OrderStatus {
            order_id: result["orderId"]
                .as_u64()
                .map(|v| v.to_string())
                .ok_or_else(|| ExchangeError::Order("missing orderId in order status".into()))?,
            symbol: result["symbol"]
                .as_str()
                .ok_or_else(|| ExchangeError::Order("missing symbol in order status".into()))?
                .to_string(),
            side: Side::from_taker(result["side"].as_str().unwrap_or("")),
            qty: result["origQty"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .map_err(|_| ExchangeError::Order("invalid origQty in order status".into()))?,
            filled_qty: result["executedQty"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .map_err(|_| ExchangeError::Order("invalid executedQty in order status".into()))?,
            price: result["price"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .map_err(|_| ExchangeError::Order("invalid price in order status".into()))?,
            avg_price: result["avgPrice"]
                .as_str()
                .unwrap_or("0")
                .parse()
                .map_err(|_| ExchangeError::Order("invalid avgPrice in order status".into()))?,
            status: result["status"]
                .as_str()
                .ok_or_else(|| ExchangeError::Order("missing status in order status".into()))?
                .to_string(),
        })
    }

    async fn account_info(&self) -> Result<AccountInfo> {
        let result = self.request("account.info", Map::new()).await?;
        Self::parse_account_info(result)
    }

    async fn current_price(&self, symbol: &str) -> Result<f64> {
        let mut params = Map::new();
        params.insert("symbol".into(), Value::String(normalize_symbol(symbol)?));
        let result = self.request("ticker.price", params).await?;
        result["price"]
            .as_str()
            .unwrap_or("0")
            .parse()
            .map_err(|_| ExchangeError::Rest("parse error".into()))
    }
}

fn validate_order_id(order_id: &str) -> Result<String> {
    let order_id = order_id.trim();
    if order_id.is_empty() || order_id.len() > 36 || !order_id.is_ascii() {
        return Err(ExchangeError::Order(
            "order ID must be a non-empty ASCII value up to 36 characters".into(),
        ));
    }
    Ok(order_id.into())
}

fn validate_client_order_id(order_id: &str) -> Result<String> {
    let order_id = order_id.trim();
    if order_id.is_empty()
        || order_id.len() > 36
        || !order_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(ExchangeError::Order(
            "client order ID must be 1-36 ASCII alphanumeric, underscore, or hyphen characters"
                .into(),
        ));
    }
    Ok(order_id.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_filters() -> filters::BinanceFilters {
        filters::BinanceFilters::from_exchange_info(&serde_json::json!({
            "symbols": [{
                "symbol": "BTCUSDT",
                "filters": [
                    {"filterType": "PRICE_FILTER", "minPrice": "0.1", "maxPrice": "1000000", "tickSize": "0.1"},
                    {"filterType": "LOT_SIZE", "minQty": "0.001", "maxQty": "100", "stepSize": "0.001"},
                    {"filterType": "MIN_NOTIONAL", "notional": "5"}
                ]
            }]
        }))
        .unwrap()
    }

    fn order(order_type: OrderType, price: Option<f64>) -> OrderRequest {
        OrderRequest {
            client_order_id: "qc_0123456789abcdef_0000000000000001".into(),
            order: Order {
                symbol: "btcusdt".into(),
                side: Side::Buy,
                qty: 0.25,
                price,
                order_type,
                reduce_only: false,
                stop_loss: None,
                take_profit: None,
            },
        }
    }

    #[test]
    fn limit_order_has_required_time_in_force_and_client_id() {
        let binance = Binance::new("key", "secret", true);
        let request = order(OrderType::Limit, Some(100.0));
        let filters = test_filters();
        let params = binance
            .order_params(&request, filters.symbol("BTCUSDT").unwrap())
            .unwrap();
        assert_eq!(params["symbol"], "BTCUSDT");
        assert_eq!(params["timeInForce"], "GTC");
        assert_eq!(params["newClientOrderId"], request.client_order_id);
    }

    #[test]
    fn invalid_order_inputs_fail_before_network_io() {
        let binance = Binance::new("key", "secret", true);
        let filters = test_filters();
        let symbol_filters = filters.symbol("BTCUSDT").unwrap();
        assert!(binance
            .order_params(&order(OrderType::Market, Some(1.0)), symbol_filters)
            .is_err());
        assert!(binance
            .order_params(&order(OrderType::Limit, None), symbol_filters)
            .is_err());
        let mut invalid_qty = order(OrderType::Market, None);
        invalid_qty.order.qty = f64::NAN;
        assert!(binance.order_params(&invalid_qty, symbol_filters).is_err());
        assert!(normalize_symbol("BTC-USDT").is_err());
        assert!(validate_order_id(" ").is_err());
        assert!(validate_client_order_id("client id with spaces").is_err());
    }

    #[test]
    fn credentials_are_required_before_connection() {
        let binance = Binance::new("", "secret", true);
        assert!(matches!(
            binance.validate_credentials(),
            Err(ExchangeError::Auth(_))
        ));
    }

    #[test]
    fn malformed_account_snapshot_is_rejected_instead_of_zeroed() {
        let snapshot = serde_json::json!({
            "assets": [{
                "asset": "USDT",
                "walletBalance": "not-a-number",
                "crossWalletBalance": "1"
            }],
            "positions": []
        });
        assert!(matches!(
            Binance::parse_account_info(snapshot),
            Err(ExchangeError::Rest(_))
        ));
    }
}
