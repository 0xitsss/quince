// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Local EVM wallet onboarding for Hyperliquid.
//!
//! The public profile is stored separately from a file-encrypted private key.
//! The private key is encrypted with AES-256-CBC and authenticated with
//! HMAC-SHA-256 (encrypt-then-MAC). The passphrase is never persisted.

use aes::Aes256;
use cbc::{Decryptor, Encryptor};
use cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use k256::ecdsa::{SigningKey, VerifyingKey};
use k256::elliptic_curve::zeroize::{Zeroize, Zeroizing};
use pbkdf2::pbkdf2_hmac;
use quince_exchange::hyperliquid::{
    execution::{HyperliquidNetwork, HyperliquidSignature, HyperliquidSigner},
    signing,
};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sha3::{Digest, Keccak256};
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

const WALLET_FORMAT_VERSION: u8 = 1;
const PBKDF2_ITERATIONS: u32 = 600_000;
const SALT_LEN: usize = 16;
const IV_LEN: usize = 16;
const KEY_LEN: usize = 32;
const DERIVED_KEY_LEN: usize = 64;
const MAC_DOMAIN: &[u8] = b"quince-wallet-v1";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletProfile {
    pub version: u8,
    pub hyperliquid_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedWalletFile {
    version: u8,
    kdf: String,
    iterations: u32,
    cipher: String,
    salt_hex: String,
    iv_hex: String,
    ciphertext_hex: String,
    mac_hex: String,
}

/// Signer backed by the encrypted wallet file. It retains a passphrase only
/// for the lifetime of this process; the decrypted signing key is zeroized
/// after every signature.
pub struct EncryptedFileHyperliquidSigner {
    address: String,
    passphrase: Zeroizing<String>,
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

fn encrypted_key_path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("wallet.enc.json"))
}

fn ensure_secure_directory(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("create wallet directory: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("secure wallet directory: {e}"))?;
    }
    Ok(())
}

fn write_private_file(path: &Path, contents: &[u8], label: &str) -> Result<(), String> {
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, contents).map_err(|e| format!("write {label}: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("secure {label}: {e}"))?;
    }
    fs::rename(&temporary, path).map_err(|e| format!("activate {label}: {e}"))
}

fn address_for_key(key: &SigningKey) -> String {
    let encoded = VerifyingKey::from(key).to_encoded_point(false);
    let digest = Keccak256::digest(&encoded.as_bytes()[1..]);
    format!("0x{}", hex::encode(&digest[12..]))
}

fn mac_input(salt: &[u8], iv: &[u8], ciphertext: &[u8], iterations: u32) -> Vec<u8> {
    let mut input =
        Vec::with_capacity(MAC_DOMAIN.len() + salt.len() + iv.len() + ciphertext.len() + 4);
    input.extend_from_slice(MAC_DOMAIN);
    input.extend_from_slice(&iterations.to_be_bytes());
    input.extend_from_slice(salt);
    input.extend_from_slice(iv);
    input.extend_from_slice(ciphertext);
    input
}

fn derive_keys(passphrase: &str, salt: &[u8], iterations: u32) -> Zeroizing<[u8; DERIVED_KEY_LEN]> {
    let mut keys = Zeroizing::new([0_u8; DERIVED_KEY_LEN]);
    pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), salt, iterations, &mut *keys);
    keys
}

fn encrypt_private_key_with_material(
    private_key: &[u8],
    passphrase: &str,
    salt: &[u8; SALT_LEN],
    iv: &[u8; IV_LEN],
) -> Result<EncryptedWalletFile, String> {
    if private_key.len() != KEY_LEN {
        return Err("wallet private key must be exactly 32 bytes".into());
    }
    let keys = derive_keys(passphrase, salt, PBKDF2_ITERATIONS);
    let mut padded = private_key.to_vec();
    let message_len = padded.len();
    padded.resize(message_len + IV_LEN, 0);
    let ciphertext = Encryptor::<Aes256>::new_from_slices(&keys[..KEY_LEN], iv)
        .map_err(|_| "initialize wallet encryption failed")?
        .encrypt_padded_mut::<Pkcs7>(&mut padded, message_len)
        .map_err(|_| "encrypt wallet private key failed")?
        .to_vec();
    padded.zeroize();

    let input = mac_input(salt, iv, &ciphertext, PBKDF2_ITERATIONS);
    let mut mac = HmacSha256::new_from_slice(&keys[KEY_LEN..])
        .map_err(|_| "initialize wallet authentication failed")?;
    mac.update(&input);
    let tag = mac.finalize().into_bytes();

    Ok(EncryptedWalletFile {
        version: WALLET_FORMAT_VERSION,
        kdf: "PBKDF2-HMAC-SHA256".into(),
        iterations: PBKDF2_ITERATIONS,
        cipher: "AES-256-CBC+HMAC-SHA256".into(),
        salt_hex: hex::encode(salt),
        iv_hex: hex::encode(iv),
        ciphertext_hex: hex::encode(ciphertext),
        mac_hex: hex::encode(tag),
    })
}

fn decode_fixed<const N: usize>(value: &str, field: &str) -> Result<[u8; N], String> {
    let mut bytes = hex::decode(value).map_err(|_| format!("wallet {field} is not hexadecimal"))?;
    let result: Result<[u8; N], String> = bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("wallet {field} has an invalid length"));
    bytes.zeroize();
    result
}

fn decrypt_private_key(
    file: &EncryptedWalletFile,
    passphrase: &str,
) -> Result<Zeroizing<Vec<u8>>, String> {
    if file.version != WALLET_FORMAT_VERSION
        || file.kdf != "PBKDF2-HMAC-SHA256"
        || file.cipher != "AES-256-CBC+HMAC-SHA256"
        || file.iterations < PBKDF2_ITERATIONS
    {
        return Err("wallet encrypted-file format is unsupported".into());
    }
    let salt = decode_fixed::<SALT_LEN>(&file.salt_hex, "salt")?;
    let iv = decode_fixed::<IV_LEN>(&file.iv_hex, "IV")?;
    let mut ciphertext = Zeroizing::new(
        hex::decode(&file.ciphertext_hex).map_err(|_| "wallet ciphertext is not hexadecimal")?,
    );
    let mac_bytes = decode_fixed::<32>(&file.mac_hex, "MAC")?;
    let keys = derive_keys(passphrase, &salt, file.iterations);
    let input = mac_input(&salt, &iv, &ciphertext, file.iterations);
    let mut mac = HmacSha256::new_from_slice(&keys[KEY_LEN..])
        .map_err(|_| "initialize wallet authentication failed")?;
    mac.update(&input);
    mac.verify_slice(&mac_bytes)
        .map_err(|_| "wallet passphrase is incorrect or encrypted file was modified")?;
    let plaintext = Decryptor::<Aes256>::new_from_slices(&keys[..KEY_LEN], &iv)
        .map_err(|_| "initialize wallet decryption failed")?
        .decrypt_padded_mut::<Pkcs7>(&mut ciphertext)
        .map_err(|_| "wallet encrypted data is invalid")?;
    if plaintext.len() != KEY_LEN {
        return Err("wallet decrypted private key has an invalid length".into());
    }
    Ok(Zeroizing::new(plaintext.to_vec()))
}

fn save_encrypted_private_key(private_key: &[u8], passphrase: &str) -> Result<(), String> {
    let dir = config_dir()?;
    ensure_secure_directory(&dir)?;
    let mut salt = [0_u8; SALT_LEN];
    let mut iv = [0_u8; IV_LEN];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut iv);
    let file = encrypt_private_key_with_material(private_key, passphrase, &salt, &iv)?;
    salt.zeroize();
    iv.zeroize();
    let contents =
        serde_json::to_vec_pretty(&file).map_err(|e| format!("serialize encrypted wallet: {e}"))?;
    write_private_file(&encrypted_key_path()?, &contents, "encrypted wallet")
}

fn signing_key_from_encrypted_file(passphrase: &str) -> Result<SigningKey, String> {
    let path = encrypted_key_path()?;
    let contents = fs::read(path).map_err(|e| format!("read encrypted wallet: {e}"))?;
    let file: EncryptedWalletFile =
        serde_json::from_slice(&contents).map_err(|e| format!("parse encrypted wallet: {e}"))?;
    let mut secret = decrypt_private_key(&file, passphrase)?;
    let key = SigningKey::from_slice(&secret).map_err(|_| "wallet private key is invalid")?;
    secret.zeroize();
    Ok(key)
}

fn save_profile(profile: &WalletProfile) -> Result<(), String> {
    let dir = config_dir()?;
    ensure_secure_directory(&dir)?;
    let contents = serde_json::to_vec_pretty(profile).map_err(|e| e.to_string())?;
    write_private_file(&profile_path()?, &contents, "wallet profile")
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
    Ok(encrypted_key_path()?.is_file())
}

fn passphrase_from_environment() -> Option<Zeroizing<String>> {
    std::env::var("QUINCE_WALLET_PASSPHRASE")
        .ok()
        .filter(|value| !value.is_empty())
        .map(Zeroizing::new)
}

fn prompt_existing_passphrase() -> Result<Zeroizing<String>, String> {
    if let Some(passphrase) = passphrase_from_environment() {
        return Ok(passphrase);
    }
    if !is_interactive() {
        return Err(
            "set QUINCE_WALLET_PASSPHRASE to unlock the encrypted wallet in non-interactive mode"
                .into(),
        );
    }
    rpassword::prompt_password("Wallet passphrase (input hidden): ")
        .map(Zeroizing::new)
        .map_err(|e| format!("read wallet passphrase: {e}"))
}

fn prompt_new_passphrase() -> Result<Zeroizing<String>, String> {
    let first = rpassword::prompt_password("New wallet passphrase (input hidden): ")
        .map_err(|e| format!("read wallet passphrase: {e}"))?;
    let second = rpassword::prompt_password("Repeat wallet passphrase: ")
        .map_err(|e| format!("read wallet passphrase: {e}"))?;
    if first.len() < 12 {
        return Err("wallet passphrase must contain at least 12 characters".into());
    }
    if first != second {
        return Err("wallet passphrases do not match".into());
    }
    Ok(Zeroizing::new(first))
}

/// Opens the encrypted-file signer only if its secret belongs to the public
/// profile. This catches a replaced encrypted file before authenticated use.
pub fn load_hyperliquid_signer() -> Result<EncryptedFileHyperliquidSigner, String> {
    let profile = load_profile()?.ok_or("Hyperliquid wallet profile is missing")?;
    let passphrase = prompt_existing_passphrase()?;
    let key = signing_key_from_encrypted_file(&passphrase)?;
    let derived = address_for_key(&key);
    if !derived.eq_ignore_ascii_case(&profile.hyperliquid_address) {
        return Err("encrypted wallet key does not match the Hyperliquid wallet profile".into());
    }
    Ok(EncryptedFileHyperliquidSigner {
        address: derived,
        passphrase,
    })
}

impl HyperliquidSigner for EncryptedFileHyperliquidSigner {
    fn address(&self) -> &str {
        &self.address
    }

    fn sign_l1_action(
        &self,
        connection_id: [u8; 32],
        network: HyperliquidNetwork,
    ) -> quince_exchange::r#trait::Result<HyperliquidSignature> {
        let key = signing_key_from_encrypted_file(&self.passphrase)
            .map_err(quince_exchange::r#trait::ExchangeError::Auth)?;
        signing::sign_l1_action(&key, connection_id, network)
            .map_err(quince_exchange::r#trait::ExchangeError::Auth)
    }
}

pub fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

pub fn needs_setup() -> Result<bool, String> {
    Ok(load_profile()?.is_none() || !has_private_key()?)
}

fn create_profile_and_store(key: &SigningKey, passphrase: &str) -> Result<WalletProfile, String> {
    let mut private_key = key.to_bytes();
    let profile = WalletProfile {
        version: 2,
        hyperliquid_address: address_for_key(key),
    };
    save_encrypted_private_key(&private_key, passphrase)?;
    private_key.zeroize();
    save_profile(&profile)?;
    Ok(profile)
}

pub fn create_wallet(passphrase: &str) -> Result<WalletProfile, String> {
    let key = SigningKey::random(&mut OsRng);
    create_profile_and_store(&key, passphrase)
}

pub fn import_wallet(raw_private_key: &str, passphrase: &str) -> Result<WalletProfile, String> {
    let mut normalized = raw_private_key
        .trim()
        .strip_prefix("0x")
        .unwrap_or(raw_private_key.trim())
        .to_owned();
    let mut bytes =
        hex::decode(&normalized).map_err(|_| "private key must be 32-byte hexadecimal")?;
    let key = SigningKey::from_slice(&bytes).map_err(|_| "invalid secp256k1 private key")?;
    let result = create_profile_and_store(&key, passphrase);
    bytes.zeroize();
    normalized.zeroize();
    result
}

/// Start a terminal-only setup wizard. Private-key and passphrase input is
/// never echoed.
pub fn run_setup_wizard() -> Result<WalletProfile, String> {
    if !is_interactive() {
        return Err("wallet setup requires an interactive terminal".into());
    }
    if let (Some(profile), true) = (load_profile()?, has_private_key()?) {
        return Ok(profile);
    }
    eprintln!("No encrypted Hyperliquid wallet is configured.");
    eprint!("Create a new wallet [c] or import a private key [i]? ");
    io::stderr().flush().map_err(|e| e.to_string())?;
    let mut choice = String::new();
    io::stdin()
        .read_line(&mut choice)
        .map_err(|e| format!("read wallet choice: {e}"))?;
    let passphrase = prompt_new_passphrase()?;
    match choice.trim().to_ascii_lowercase().as_str() {
        "c" | "create" => create_wallet(&passphrase),
        "i" | "import" => {
            let mut private_key = rpassword::prompt_password("Private key (input hidden): ")
                .map_err(|e| format!("read private key: {e}"))?;
            let result = import_wallet(&private_key, &passphrase);
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
    fn encrypted_wallet_round_trip() {
        let private_key = [7_u8; KEY_LEN];
        let encrypted = encrypt_private_key_with_material(
            &private_key,
            "this is a sufficiently long passphrase",
            &[1_u8; SALT_LEN],
            &[2_u8; IV_LEN],
        )
        .unwrap();
        let decrypted =
            decrypt_private_key(&encrypted, "this is a sufficiently long passphrase").unwrap();
        assert_eq!(decrypted.as_slice(), private_key);
        assert_ne!(encrypted.ciphertext_hex, hex::encode(private_key));
    }

    #[test]
    fn encrypted_wallet_rejects_wrong_passphrase_and_tampering() {
        let private_key = [9_u8; KEY_LEN];
        let mut encrypted = encrypt_private_key_with_material(
            &private_key,
            "this is a sufficiently long passphrase",
            &[3_u8; SALT_LEN],
            &[4_u8; IV_LEN],
        )
        .unwrap();
        assert!(decrypt_private_key(&encrypted, "wrong passphrase").is_err());
        encrypted.ciphertext_hex.replace_range(0..2, "00");
        assert!(decrypt_private_key(&encrypted, "this is a sufficiently long passphrase").is_err());
    }

    #[test]
    fn invalid_private_key_is_rejected() {
        assert!(SigningKey::from_slice(&[0_u8; 32]).is_err());
    }

    #[test]
    fn l1_signature_vector_uses_the_expected_wallet_key() {
        let key = SigningKey::from_slice(
            &hex::decode("e908f86dbb4d55ac876378565aafeabc187f6690f046459397b17d9b9a19688e")
                .unwrap(),
        )
        .unwrap();
        let id: [u8; 32] =
            hex::decode("de6c4037798a4434ca03cd05f00e3b803126221375cd1e7eaaaf041768be06eb")
                .unwrap()
                .try_into()
                .unwrap();
        let signature = signing::sign_l1_action(&key, id, HyperliquidNetwork::Testnet).unwrap();
        assert_eq!(signature.v, 28);
        assert_eq!(
            signature.r,
            "0x1713c0fc661b792a50e8ffdd59b637b1ed172d9a3aa4d801d9d88646710fb74b"
        );
    }
}
