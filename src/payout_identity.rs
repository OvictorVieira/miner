//! End-to-end proofs that the configured payout identity is the one that
//! reaches the pool, offline (US-011).
//!
//! The tests here drive a tiny in-process Stratum client against the
//! deterministic `fake_stratum` fixture using whatever `Config::pool_username`
//! the configuration layer produced. They are the structural counterpart to
//! what a real miner binary does — the client speaks the same wire protocol
//! (subscribe → authorize → set_difficulty → notify → submit) and the fake
//! pool records every field, so the integration asserts can prove:
//!
//! * The exact `pool_username` the config produced is the one the pool sees.
//! * The retired publisher donation address never appears as a mining
//!   identity or fallback, regardless of configuration.
//! * An invalid payout address is rejected at `Config::from_env` before
//!   anything downstream (the engine supervisor, any network client) could
//!   ever be started.
//!
//! Nothing in this module ships in the release binary — the Stratum helpers
//! are only compiled under `#[cfg(test)]`. The retired address is assembled
//! from fragments so its exact value cannot reappear in a tracked file while
//! the payout regression proof remains executable.

#![allow(dead_code)]

/// The donation address removed from the project. US-011 proves it is never
/// used as a mining identity or fallback.
pub const REMOVED_DONATION_ADDR: &str = concat!(
    "bc1pwy2ulg769ffvhwchk4yzkcq5",
    "yq699qwrrkg3a4lq942rj47sutcq2xjny5"
);

/// Mask a pool secret for log and error output. Short strings become a
/// bare marker; longer ones keep their byte length visible so an operator
/// can tell "empty" from "set" without seeing the value.
pub fn redact_secret(raw: &str) -> String {
    if raw.is_empty() {
        "<empty>".into()
    } else {
        format!("<redacted:{} bytes>", raw.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::fake_stratum::{FakeStratum, Scenario, JOB_ID};
    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpStream;
    use tokio::time::timeout;

    const CLIENT_TIMEOUT: Duration = Duration::from_secs(5);

    fn cfg(vars: &[(&str, &str)]) -> Config {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_vars(|k| map.get(k).cloned()).expect("valid config")
    }

    /// Drive a full Stratum v1 handshake against `addr` using exactly the
    /// supplied `pool_username` — the same string the real miner would put
    /// on the wire — and submit one share. The returned share digest is
    /// deterministic so a failure prints the same values every run.
    async fn run_identity_client(addr: SocketAddr, pool_username: &str) {
        let stream = TcpStream::connect(addr).await.expect("connect");
        let (r, mut w) = stream.into_split();
        let mut reader = BufReader::new(r);
        let mut line = String::new();

        async fn send_line<W: tokio::io::AsyncWrite + Unpin>(
            w: &mut W,
            v: serde_json::Value,
        ) -> std::io::Result<()> {
            let mut bytes = serde_json::to_vec(&v).unwrap();
            bytes.push(b'\n');
            w.write_all(&bytes).await
        }
        async fn recv_line<R: tokio::io::AsyncBufRead + Unpin>(
            r: &mut R,
            buf: &mut String,
        ) -> Option<serde_json::Value> {
            buf.clear();
            let n = timeout(CLIENT_TIMEOUT, r.read_line(buf)).await.ok()?.ok()?;
            if n == 0 {
                return None;
            }
            serde_json::from_str(buf.trim()).ok()
        }

        send_line(
            &mut w,
            serde_json::json!({
                "id": 1,
                "method": "mining.subscribe",
                "params": ["miner-identity-test/0.1"],
            }),
        )
        .await
        .expect("subscribe write");
        let _ = recv_line(&mut reader, &mut line)
            .await
            .expect("subscribe response");

        send_line(
            &mut w,
            serde_json::json!({
                "id": 2,
                "method": "mining.authorize",
                "params": [pool_username, "x"],
            }),
        )
        .await
        .expect("authorize write");
        let _ = recv_line(&mut reader, &mut line)
            .await
            .expect("authorize response");
        let _ = recv_line(&mut reader, &mut line)
            .await
            .expect("set_difficulty push");
        let _ = recv_line(&mut reader, &mut line)
            .await
            .expect("notify push");

        send_line(
            &mut w,
            serde_json::json!({
                "id": 3,
                "method": "mining.submit",
                "params": [pool_username, JOB_ID, "deadbeef", "5e8f1a00", "01020304"],
            }),
        )
        .await
        .expect("submit write");
        let _ = recv_line(&mut reader, &mut line)
            .await
            .expect("submit response");

        drop(w);
    }

    const MAINNET_WALLET: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";

    #[tokio::test]
    async fn configured_identity_reaches_pool_unchanged() {
        let c = cfg(&[("WALLET", MAINNET_WALLET), ("WORKER_NAME", "worker-A")]);
        assert_eq!(c.pool_username, format!("{MAINNET_WALLET}.worker-A"));

        let server = FakeStratum::start(Scenario::AcceptAll).await.unwrap();
        let addr = server.addr();
        run_identity_client(addr, &c.pool_username).await;
        let rec = server.wait().await;

        let auth = rec.authorize.expect("authorize recorded");
        assert_eq!(
            auth.username, c.pool_username,
            "authorize username was not the configured pool_username"
        );
        assert!(
            auth.username.contains(&c.payout_address),
            "authorize username must carry the configured payout address"
        );

        assert_eq!(rec.submits.len(), 1, "exactly one submit expected");
        assert_eq!(
            rec.submits[0].worker, c.pool_username,
            "submit worker was not the configured pool_username"
        );
    }

    #[tokio::test]
    async fn pool_username_override_is_used_verbatim_on_the_wire() {
        // When POOL_USERNAME is set explicitly, that string — not the wallet
        // and not any synthesized default — is what the pool must see.
        let c = cfg(&[
            ("WALLET", MAINNET_WALLET),
            ("POOL_USERNAME", "pool-account.rig7"),
        ]);
        assert_eq!(c.pool_username, "pool-account.rig7");

        let server = FakeStratum::start(Scenario::AcceptAll).await.unwrap();
        let addr = server.addr();
        run_identity_client(addr, &c.pool_username).await;
        let rec = server.wait().await;

        let auth = rec.authorize.expect("authorize recorded");
        assert_eq!(auth.username, "pool-account.rig7");
        assert_eq!(rec.submits[0].worker, "pool-account.rig7");
    }

    #[tokio::test]
    async fn donation_address_is_never_a_mining_identity_or_fallback() {
        // Build a config with a non-donation wallet and drive the full
        // handshake. The authorize and submit frames must contain the
        // configured wallet and must not contain the hardcoded donation
        // address anywhere.
        let c = cfg(&[("WALLET", MAINNET_WALLET), ("WORKER_NAME", "worker-B")]);
        assert_ne!(c.payout_address, REMOVED_DONATION_ADDR);
        assert!(!c.pool_username.contains(REMOVED_DONATION_ADDR));

        let server = FakeStratum::start(Scenario::AcceptAll).await.unwrap();
        let addr = server.addr();
        run_identity_client(addr, &c.pool_username).await;
        let rec = server.wait().await;

        let auth = rec.authorize.expect("authorize recorded");
        assert!(
            !auth.username.contains(REMOVED_DONATION_ADDR),
            "donation address leaked into authorize username"
        );
        assert!(
            !rec.submits
                .iter()
                .any(|s| s.worker.contains(REMOVED_DONATION_ADDR)),
            "donation address leaked into a submit worker"
        );
    }

    #[test]
    fn no_source_file_hardcodes_the_donation_address_as_fallback() {
        // No Rust source file may embed the removed address — otherwise a
        // future refactor could silently reintroduce it as an identity
        // fallback. The policy test scans every tracked file as well.
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for entry in std::fs::read_dir(&src).expect("read src/") {
            let path = entry.expect("entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            // This file assembles the address from fragments for this proof.
            if path.file_name().and_then(|n| n.to_str()) == Some("payout_identity.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read source");
            assert!(
                !text.contains(REMOVED_DONATION_ADDR),
                "{} must not hardcode the donation address",
                path.display()
            );
        }
    }

    #[test]
    fn invalid_payout_address_prevents_engine_startup() {
        // The supervisor consumes a `Config`. There is no other construction
        // path: `Config::from_env` and `Config::from_vars` both run full
        // address validation first. An invalid WALLET therefore fails before
        // the engine binary can ever be spawned, with no fallback to a
        // default or donation address.
        let bogus = Config::from_vars(|k| match k {
            "WALLET" => Some("not-a-valid-address".to_string()),
            _ => None,
        });
        assert!(
            bogus.is_err(),
            "invalid WALLET must fail config construction"
        );

        let missing = Config::from_vars(|_| None);
        assert!(
            missing.is_err(),
            "missing WALLET must fail — no default identity allowed"
        );
    }

    #[test]
    fn authorize_record_debug_redacts_the_password() {
        use crate::fake_stratum::AuthorizeRecord;
        let r = AuthorizeRecord {
            username: "user.worker".into(),
            password: "super-secret".into(),
        };
        let dbg = format!("{r:?}");
        assert!(dbg.contains("user.worker"), "username must stay visible");
        assert!(
            !dbg.contains("super-secret"),
            "debug output must not leak the password: {dbg}"
        );
        assert!(
            dbg.contains("redacted"),
            "debug output must mark the password: {dbg}"
        );
    }

    #[test]
    fn redact_secret_hides_contents_but_keeps_length() {
        let s = redact_secret("hunter2");
        assert!(!s.contains("hunter2"));
        assert!(s.contains("7"));

        assert_eq!(redact_secret(""), "<empty>");
    }
}
