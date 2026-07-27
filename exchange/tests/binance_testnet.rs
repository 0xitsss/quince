// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Opt-in, read-only Binance Futures testnet connectivity checks.
//!
//! Run explicitly with restricted testnet credentials:
//! `BINANCE_TESTNET_API_KEY=... BINANCE_TESTNET_SECRET_KEY=... cargo test -p quince-exchange --test binance_testnet -- --ignored`
//!
//! The suite subscribes and requests account information only. It never places,
//! amends, or cancels an order.

use quince_exchange::binance::Binance;
use quince_exchange::r#trait::Exchange;

fn required_env(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} must be set for the Binance testnet check"))
}

#[tokio::test]
#[ignore = "requires explicitly supplied Binance Futures testnet credentials and network access"]
async fn authenticated_testnet_account_snapshot_is_readable() {
    let exchange = Binance::new(
        &required_env("BINANCE_TESTNET_API_KEY"),
        &required_env("BINANCE_TESTNET_SECRET_KEY"),
        true,
    );
    let symbols = vec!["BTCUSDT".to_string()];
    let _stream = exchange
        .subscribe(&symbols)
        .await
        .expect("testnet subscribe");
    let account = exchange.account_info().await.expect("testnet account info");
    assert!(
        account
            .balances
            .iter()
            .all(|balance| balance.wallet.is_finite()),
        "testnet account balances must be finite"
    );
}
