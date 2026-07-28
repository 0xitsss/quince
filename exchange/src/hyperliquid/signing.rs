// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! Minimal, vector-tested primitives for Hyperliquid L1-action signatures.
//!
//! This intentionally covers only the EIP-712 envelope around an already
//! canonical `connection_id`. Action MessagePack encoding belongs in a separate
//! module and must earn the same test-vector coverage before live submission.

use k256::ecdsa::{RecoveryId, Signature, SigningKey};
use sha3::{Digest, Keccak256};

use super::execution::{HyperliquidNetwork, HyperliquidSignature};

const DOMAIN_TYPE: &str =
    "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)";
const AGENT_TYPE: &str = "Agent(string source,bytes32 connectionId)";

fn keccak(bytes: impl AsRef<[u8]>) -> [u8; 32] {
    Keccak256::digest(bytes).into()
}

fn word_from_hash(hash: [u8; 32]) -> [u8; 32] {
    hash
}

fn domain_separator() -> [u8; 32] {
    let mut encoded = Vec::with_capacity(32 * 5);
    encoded.extend(keccak(DOMAIN_TYPE));
    encoded.extend(keccak("Exchange"));
    encoded.extend(keccak("1"));
    let mut chain_id = [0_u8; 32];
    chain_id[24..].copy_from_slice(&1337_u64.to_be_bytes());
    encoded.extend(chain_id);
    encoded.extend([0_u8; 32]); // verifyingContract = address(0)
    keccak(encoded)
}

/// Returns the EIP-712 digest the wallet must sign for a canonical L1 action
/// connection ID. The protocol uses source `a` on mainnet and `b` on testnet.
pub fn l1_action_signing_digest(connection_id: [u8; 32], network: HyperliquidNetwork) -> [u8; 32] {
    let source = match network {
        HyperliquidNetwork::Mainnet => "a",
        HyperliquidNetwork::Testnet => "b",
    };
    let mut agent = Vec::with_capacity(64);
    agent.extend(keccak(AGENT_TYPE));
    agent.extend(word_from_hash(keccak(source)));
    agent.extend(connection_id);

    let mut payload = Vec::with_capacity(66);
    payload.extend([0x19, 0x01]);
    payload.extend(domain_separator());
    payload.extend(keccak(agent));
    keccak(payload)
}

/// Signs a canonical L1 action connection ID using Ethereum's `r || s || v`
/// shape. `v` is normalized to 27 or 28 for the exchange API.
pub fn sign_l1_action(
    signing_key: &SigningKey,
    connection_id: [u8; 32],
    network: HyperliquidNetwork,
) -> Result<HyperliquidSignature, String> {
    let digest = l1_action_signing_digest(connection_id, network);
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(&digest)
        .map_err(|error| format!("sign Hyperliquid L1 action: {error}"))?;
    Ok(signature_parts(signature, recovery_id))
}

fn signature_parts(signature: Signature, recovery_id: RecoveryId) -> HyperliquidSignature {
    let bytes = signature.to_bytes();
    HyperliquidSignature {
        r: format!("0x{}", hex::encode(&bytes[..32])),
        s: format!("0x{}", hex::encode(&bytes[32..])),
        v: 27 + recovery_id.to_byte(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l1_signature_matches_the_official_sdk_vector() {
        let key = SigningKey::from_slice(
            &hex::decode("e908f86dbb4d55ac876378565aafeabc187f6690f046459397b17d9b9a19688e")
                .unwrap(),
        )
        .unwrap();
        let connection_id: [u8; 32] =
            hex::decode("de6c4037798a4434ca03cd05f00e3b803126221375cd1e7eaaaf041768be06eb")
                .unwrap()
                .try_into()
                .unwrap();

        let mainnet = sign_l1_action(&key, connection_id, HyperliquidNetwork::Mainnet).unwrap();
        assert_eq!(format!("{}{}{:02x}", mainnet.r, &mainnet.s[2..], mainnet.v), "0xfa8a41f6a3fa728206df80801a83bcbfbab08649cd34d9c0bfba7c7b2f99340f53a00226604567b98a1492803190d65a201d6805e5831b7044f17fd530aec7841c");

        let testnet = sign_l1_action(&key, connection_id, HyperliquidNetwork::Testnet).unwrap();
        assert_eq!(format!("{}{}{:02x}", testnet.r, &testnet.s[2..], testnet.v), "0x1713c0fc661b792a50e8ffdd59b637b1ed172d9a3aa4d801d9d88646710fb74b33959f4d075a7ccbec9f2374a6da21ffa4448d58d0413a0d335775f680a881431c");
    }
}
