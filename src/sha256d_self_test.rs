//! Known-answer SHA-256d self-test for the CPU engine.
//!
//! Hashes real Bitcoin mainnet block headers and verifies the resulting
//! double-SHA256 digests match the on-chain values. A mismatch means the
//! shipped engine would not produce real Bitcoin hashes, so the test exits
//! nonzero and the application refuses to start mining.
//!
//! The test is pool-free: all inputs and expected outputs are compiled in.

use sha2::{Digest, Sha256};

struct Vector {
    name: &'static str,
    header_hex: &'static str,
    // Little-endian SHA-256d of the 80-byte header. This is the raw digest
    // bytes; the big-endian block hash shown by explorers is this reversed.
    expected_digest_le_hex: &'static str,
}

const VECTORS: &[Vector] = &[
    Vector {
        name: "bitcoin mainnet block 0 (genesis)",
        header_hex: concat!(
            "01000000",
            "0000000000000000000000000000000000000000000000000000000000000000",
            "3ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a",
            "29ab5f49",
            "ffff001d",
            "1dac2b7c",
        ),
        expected_digest_le_hex: "6fe28c0ab6f1b372c1a6a246ae63f74f931e8365e15a089c68d6190000000000",
    },
    Vector {
        name: "bitcoin mainnet block 1",
        header_hex: concat!(
            "01000000",
            "6fe28c0ab6f1b372c1a6a246ae63f74f931e8365e15a089c68d6190000000000",
            "982051fd1e4ba744bbbe680e1fee14677ba1a3c3540bf7b1cdb606e857233e0e",
            "61bc6649",
            "ffff001d",
            "01e36299",
        ),
        expected_digest_le_hex: "4860eb18bf1b1620e37e9490fc8a427514416fd75159ab86688e9a8300000000",
    },
];

pub(crate) fn sha256d(bytes: &[u8]) -> [u8; 32] {
    let first = Sha256::digest(bytes);
    Sha256::digest(first).into()
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    if !hex.len().is_multiple_of(2) {
        return Err(format!("odd hex length: {}", hex.len()));
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for i in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[i..i + 2], 16)
            .map_err(|e| format!("invalid hex at offset {i}: {e}"))?;
        out.push(byte);
    }
    Ok(out)
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Hash every pinned Bitcoin header and verify the digests. Returns the first
/// mismatch (if any) as a human-readable error.
pub(crate) fn run() -> Result<(), String> {
    if VECTORS.len() < 2 {
        return Err("self-test requires at least two independent header vectors".into());
    }
    for v in VECTORS {
        let header = decode_hex(v.header_hex)
            .map_err(|e| format!("{}: cannot decode header: {e}", v.name))?;
        if header.len() != 80 {
            return Err(format!(
                "{}: header length {} != 80 bytes",
                v.name,
                header.len()
            ));
        }
        let expected = decode_hex(v.expected_digest_le_hex)
            .map_err(|e| format!("{}: cannot decode expected digest: {e}", v.name))?;
        if expected.len() != 32 {
            return Err(format!(
                "{}: expected digest length {} != 32 bytes",
                v.name,
                expected.len()
            ));
        }
        let actual = sha256d(&header);
        if actual.as_slice() != expected.as_slice() {
            return Err(format!(
                "{}: SHA-256d mismatch\n  expected (LE): {}\n  actual   (LE): {}",
                v.name,
                v.expected_digest_le_hex,
                hex_encode(&actual),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_test_passes_on_pinned_vectors() {
        run().expect("self-test must pass on real Bitcoin headers");
    }

    #[test]
    fn vectors_include_at_least_two_distinct_headers() {
        assert!(VECTORS.len() >= 2, "need at least two independent vectors");
        let a = decode_hex(VECTORS[0].header_hex).unwrap();
        let b = decode_hex(VECTORS[1].header_hex).unwrap();
        assert_ne!(a, b, "vectors must be independent");
    }

    #[test]
    fn every_pinned_header_is_exactly_80_bytes() {
        for v in VECTORS {
            let header = decode_hex(v.header_hex).unwrap();
            assert_eq!(header.len(), 80, "{} header is not 80 bytes", v.name);
        }
    }

    #[test]
    fn sha256d_matches_known_empty_input_vector() {
        // SHA-256d("") = SHA-256(SHA-256("")) — fixed value in the sha2 spec.
        let d = sha256d(b"");
        let expected =
            decode_hex("5df6e0e2761359d30a8275058e299fcc0381534545f55cf43e41983f5d4c9456").unwrap();
        assert_eq!(d.as_slice(), expected.as_slice());
    }

    #[test]
    fn tampered_header_bit_flips_digest() {
        let v = &VECTORS[0];
        let mut header = decode_hex(v.header_hex).unwrap();
        let good = sha256d(&header);
        header[0] ^= 0x01;
        let bad = sha256d(&header);
        assert_ne!(good, bad, "a single bit flip must change SHA-256d output");
    }

    #[test]
    fn genesis_digest_reverses_to_known_big_endian_block_hash() {
        let v = &VECTORS[0];
        let mut digest = sha256d(&decode_hex(v.header_hex).unwrap());
        digest.reverse();
        assert_eq!(
            hex_encode(&digest),
            "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
        );
    }

    #[test]
    fn decode_hex_rejects_odd_length() {
        assert!(decode_hex("abc").is_err());
    }

    #[test]
    fn decode_hex_rejects_non_hex_chars() {
        assert!(decode_hex("zz").is_err());
    }
}
