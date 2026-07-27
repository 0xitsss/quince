// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Local EVM wallet onboarding for Hyperliquid.
//!
//! Private keys are stored only in the operating system credential store. The
//! local profile contains the public address and no signing secret.

use k256::ecdsa::{SigningKey, VerifyingKey};
use k256::elliptic_curve::zeroize::Zeroize;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "quince.hyperliquid";
const KEYRING_ACCOUNT: &str = "default";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletProfile {
    pub version: u8,
    pub hyperliquid_address: String,
}

fn config_dir() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("quince"));
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set; cannot locate wallet profile")?;
    Ok(PathBuf::from(home).join(".config/quince"))
}

fn profile_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("wallet.json"))
}

fn keyring_entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT).map_err(|e| e.to_string())
}

fn address_for_key(key: &SigningKey) -> String {
    let encoded = VerifyingKey::from(key).to_encoded_point(false);
    let digest = Keccak256::digest(&encoded.as_bytes()[1..]);
    format!("0x{}", hex::encode(&digest[12..]))
}

fn save_profile(profile: &WalletProfile) -> Result<(), String> {
    let dir = config_dir()?;
    fs::create_dir_all(&dir).map_err(|e| format!("create wallet directory: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("secure wallet directory: {e}"))?;
    }
    let path = profile_path()?;
    let temporary = path.with_extension("json.tmp");
    let contents = serde_json::to_vec_pretty(profile).map_err(|e| e.to_string())?;
    fs::write(&temporary, contents).map_err(|e| format!("write wallet profile: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("secure wallet profile: {e}"))?;
    }
    fs::rename(&temporary, &path).map_err(|e| format!("activate wallet profile: {e}"))
}

pub fn load_profile() -> Result<Option<WalletProfile>, String> {
    let path = profile_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read(path).map_err(|e| format!("read wallet profile: {e}"))?;
    serde_json::from_slice(&contents)
        .map(Some)
        .map_err(|e| format!("parse wallet profile: {e}"))
}

pub fn has_private_key() -> Result<bool, String> {
    match keyring_entry()?.get_password() {
        Ok(value) => Ok(!value.is_empty()),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(error) => Err(format!("read system keychain: {error}")),
    }
}

pub fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

pub fn needs_setup() -> Result<bool, String> {
    Ok(load_profile()?.is_none() || !has_private_key()?)
}

pub fn create_wallet() -> Result<WalletProfile, String> {
    let key = SigningKey::random(&mut OsRng);
    let mut private_key = hex::encode(key.to_bytes());
    let profile = WalletProfile {
        version: 1,
        hyperliquid_address: address_for_key(&key),
    };
    let result = keyring_entry()?
        .set_password(&private_key)
        .map_err(|e| format!("store private key in system keychain: {e}"));
    private_key.zeroize();
    result?;
    save_profile(&profile)?;
    Ok(profile)
}

pub fn import_wallet(raw_private_key: &str) -> Result<WalletProfile, String> {
    let mut normalized = raw_private_key
        .trim()
        .strip_prefix("0x")
        .unwrap_or(raw_private_key.trim())
        .to_owned();
    let mut bytes =
        hex::decode(&normalized).map_err(|_| "private key must be 32-byte hexadecimal")?;
    let key = SigningKey::from_slice(&bytes).map_err(|_| "invalid secp256k1 private key")?;
    let profile = WalletProfile {
        version: 1,
        hyperliquid_address: address_for_key(&key),
    };
    let result = keyring_entry()?
        .set_password(&normalized)
        .map_err(|e| format!("store private key in system keychain: {e}"));
    bytes.zeroize();
    normalized.zeroize();
    result?;
    save_profile(&profile)?;
    Ok(profile)
}

/// Start a terminal-only setup wizard. Private-key input is never echoed.
pub fn run_setup_wizard() -> Result<WalletProfile, String> {
    if !is_interactive() {
        return Err("wallet setup requires an interactive terminal".into());
    }
    if let (Some(profile), true) = (load_profile()?, has_private_key()?) {
        return Ok(profile);
    }
    eprintln!("No Hyperliquid wallet is configured.");
    eprint!("Create a new wallet [c] or import a private key [i]? ");
    io::stderr().flush().map_err(|e| e.to_string())?;
    let mut choice = String::new();
    io::stdin()
        .read_line(&mut choice)
        .map_err(|e| format!("read wallet choice: {e}"))?;
    match choice.trim().to_ascii_lowercase().as_str() {
        "c" | "create" => create_wallet(),
        "i" | "import" => {
            let mut private_key = rpassword::prompt_password("Private key (input hidden): ")
                .map_err(|e| format!("read private key: {e}"))?;
            let result = import_wallet(&private_key);
            private_key.zeroize();
            result
        }
        _ => Err("wallet setup cancelled".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_is_evm_shaped() {
        let key = SigningKey::from_slice(&[1_u8; 32]).unwrap();
        let address = address_for_key(&key);
        assert!(address.starts_with("0x"));
        assert_eq!(address.len(), 42);
    }

    #[test]
    fn invalid_private_key_is_rejected() {
        assert!(SigningKey::from_slice(&[0_u8; 32]).is_err());
    }
}
