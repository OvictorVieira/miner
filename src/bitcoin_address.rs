//! Validates Bitcoin payout addresses without ever needing a private key.
//!
//! Supports the two address families a payout-only config can hold:
//! legacy Base58Check (P2PKH/P2SH) and Bech32/Bech32m (segwit v0/taproot).
//! There is intentionally no WIF, xprv, or seed-phrase decoding path here —
//! `Config` has no field for one (see `config.rs`), so this module only
//! ever needs to recognize *public* address encodings.

use sha2::{Digest, Sha256};

use crate::config::BitcoinNetwork;

const BASE58_ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

const MAINNET_P2PKH_VERSION: u8 = 0x00;
const MAINNET_P2SH_VERSION: u8 = 0x05;
const TESTNET_P2PKH_VERSION: u8 = 0x6f;
const TESTNET_P2SH_VERSION: u8 = 0xc4;

/// Validates `address` as a public payout address for `network`.
pub fn validate(address: &str, network: BitcoinNetwork) -> Result<(), String> {
    if address.is_empty() {
        return Err("payout address is empty".into());
    }
    if address.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("payout address contains whitespace or control characters".into());
    }

    match bech32::segwit::decode(address) {
        Ok((hrp, _witness_version, _program)) => validate_segwit_network(hrp, network),
        Err(bech32_err) => match decode_base58check(address) {
            Ok(payload) => validate_base58_network(&payload, network),
            Err(base58_err) => Err(format!(
                "not a valid Base58Check, Bech32, or Bech32m address for this network \
                 (bech32: {bech32_err}; base58: {base58_err})"
            )),
        },
    }
}

fn validate_segwit_network(hrp: bech32::Hrp, network: BitcoinNetwork) -> Result<(), String> {
    let matches = match network {
        BitcoinNetwork::Mainnet => hrp.is_valid_on_mainnet(),
        BitcoinNetwork::Testnet => hrp.is_valid_on_testnet(),
    };
    if matches {
        Ok(())
    } else {
        Err(format!(
            "bech32 human-readable prefix does not match configured network ({network:?})"
        ))
    }
}

fn validate_base58_network(payload: &[u8], network: BitcoinNetwork) -> Result<(), String> {
    let version = payload[0];
    let expected = match network {
        BitcoinNetwork::Mainnet => [MAINNET_P2PKH_VERSION, MAINNET_P2SH_VERSION],
        BitcoinNetwork::Testnet => [TESTNET_P2PKH_VERSION, TESTNET_P2SH_VERSION],
    };
    if expected.contains(&version) {
        Ok(())
    } else {
        Err(format!(
            "base58 version byte 0x{version:02x} does not match configured network ({network:?})"
        ))
    }
}

/// Decodes a Base58Check string, verifies its checksum, and returns the
/// `version || hash` payload (without the trailing 4-byte checksum).
///
/// Only the 25-byte P2PKH/P2SH payload length is accepted; this also
/// rejects WIF private keys (37/38 bytes) and extended keys (78 bytes),
/// which decode to the same alphabet but a different length.
fn decode_base58check(s: &str) -> Result<Vec<u8>, String> {
    let decoded = decode_base58(s)?;
    if decoded.len() != 25 {
        return Err(format!(
            "decoded length {} is not a valid P2PKH/P2SH payload length (25)",
            decoded.len()
        ));
    }
    let (payload, checksum) = decoded.split_at(21);
    let hash1 = Sha256::digest(payload);
    let hash2 = Sha256::digest(hash1);
    if &hash2[..4] != checksum {
        return Err("checksum mismatch".into());
    }
    Ok(payload.to_vec())
}

fn decode_base58(s: &str) -> Result<Vec<u8>, String> {
    let mut bytes: Vec<u8> = vec![0];
    for c in s.chars() {
        let digit = BASE58_ALPHABET
            .iter()
            .position(|&b| b as char == c)
            .ok_or_else(|| format!("invalid base58 character {c:?}"))? as u32;
        let mut carry = digit;
        for byte in bytes.iter_mut().rev() {
            let value = *byte as u32 * 58 + carry;
            *byte = (value & 0xff) as u8;
            carry = value >> 8;
        }
        while carry > 0 {
            bytes.insert(0, (carry & 0xff) as u8);
            carry >>= 8;
        }
    }

    let leading_ones = s.chars().take_while(|&c| c == '1').count();
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    let mut result = vec![0u8; leading_ones];
    result.extend_from_slice(&bytes[start..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_p2pkh_is_valid() {
        assert!(validate(
            "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2",
            BitcoinNetwork::Mainnet
        )
        .is_ok());
    }

    #[test]
    fn mainnet_p2sh_is_valid() {
        assert!(validate(
            "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy",
            BitcoinNetwork::Mainnet
        )
        .is_ok());
    }

    #[test]
    fn mainnet_bech32_segwit_v0_is_valid() {
        assert!(validate(
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
            BitcoinNetwork::Mainnet
        )
        .is_ok());
    }

    #[test]
    fn mainnet_bech32m_taproot_is_valid() {
        assert!(validate(
            "bc1p5d7rjq7g6rdk2yhzks9smlaqtedr4dekq08ge8ztwac72sfr9rusxg3297",
            BitcoinNetwork::Mainnet
        )
        .is_ok());
    }

    #[test]
    fn testnet_p2pkh_is_valid() {
        assert!(validate(
            "mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn",
            BitcoinNetwork::Testnet
        )
        .is_ok());
    }

    #[test]
    fn testnet_p2sh_is_valid() {
        assert!(validate(
            "2MzQwSSnBHWHqSAqtTVQ6v47XtaisrJa1Vc",
            BitcoinNetwork::Testnet
        )
        .is_ok());
    }

    #[test]
    fn testnet_bech32_segwit_v0_is_valid() {
        assert!(validate(
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx",
            BitcoinNetwork::Testnet
        )
        .is_ok());
    }

    #[test]
    fn mainnet_address_rejected_on_testnet() {
        assert!(validate(
            "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2",
            BitcoinNetwork::Testnet
        )
        .is_err());
    }

    #[test]
    fn testnet_address_rejected_on_mainnet() {
        assert!(validate(
            "mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn",
            BitcoinNetwork::Mainnet
        )
        .is_err());
    }

    #[test]
    fn mainnet_bech32_rejected_on_testnet() {
        assert!(validate(
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
            BitcoinNetwork::Testnet
        )
        .is_err());
    }

    #[test]
    fn invalid_base58_checksum_is_rejected() {
        assert!(validate(
            "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN3",
            BitcoinNetwork::Mainnet
        )
        .is_err());
    }

    #[test]
    fn invalid_bech32_checksum_is_rejected() {
        assert!(validate(
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t5",
            BitcoinNetwork::Mainnet
        )
        .is_err());
    }

    #[test]
    fn bech32_wrong_checksum_algorithm_is_rejected() {
        // Valid bech32 (not bech32m) checksum applied to a taproot (v1) program;
        // BIP-350 requires bech32m for witness version 1, so this must fail.
        assert!(validate(
            "bc1p4w46h2at4w46h2at4w46h2at4w46h2at4w46h2at4w46h2at4w4sw4k9xs",
            BitcoinNetwork::Mainnet
        )
        .is_err());
    }

    #[test]
    fn whitespace_is_rejected() {
        assert!(validate(
            " 1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2",
            BitcoinNetwork::Mainnet
        )
        .is_err());
        assert!(validate(
            "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2 ",
            BitcoinNetwork::Mainnet
        )
        .is_err());
        assert!(validate(
            "1BvBMSEY stWetqTFn5Au4m4GFg7xJaNVN2",
            BitcoinNetwork::Mainnet
        )
        .is_err());
    }

    #[test]
    fn control_characters_are_rejected() {
        assert!(validate(
            "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2\0",
            BitcoinNetwork::Mainnet
        )
        .is_err());
        assert!(validate(
            "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2\n",
            BitcoinNetwork::Mainnet
        )
        .is_err());
    }

    #[test]
    fn empty_address_is_rejected() {
        assert!(validate("", BitcoinNetwork::Mainnet).is_err());
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(validate("not-a-bitcoin-address", BitcoinNetwork::Mainnet).is_err());
    }

    #[test]
    fn wif_length_payload_is_rejected() {
        // A base58check-encoded 38-byte WIF-shaped payload must not pass as an address,
        // regardless of checksum validity, because the length (25) check runs first.
        let fake_wif = "5HueCGU8rMjxEXxiPuD5BDku4MkFqeZyd4dZ1jvhTVqvbTLvyTJ";
        assert!(validate(fake_wif, BitcoinNetwork::Mainnet).is_err());
    }
}
