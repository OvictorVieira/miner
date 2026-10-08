use std::cmp::max;
use std::path::Path;

/// Fixed MVP pool endpoint. `POOL_URL` can still override it; full syntax
/// validation of that override is added by US-071 and the override remains
/// an advanced, undocumented-for-beginners knob until then.
const DEFAULT_POOL_URL: &str = "stratum+tcp://public-pool.io:21496";

/// Mining topology. Only `Solo` is implemented in this MVP; `Shared` is
/// modeled now so a future pool-account flow has a typed home, but selecting
/// it today is an explicit, documented error rather than a silent fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiningMode {
    Solo,
    Shared,
}

impl MiningMode {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "solo" => Ok(MiningMode::Solo),
            "shared" => Ok(MiningMode::Shared),
            other => Err(format!("MODE must be 'solo' or 'shared', got {other:?}")),
        }
    }
}

/// Bitcoin network the payout address belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitcoinNetwork {
    Mainnet,
    Testnet,
}

impl BitcoinNetwork {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "mainnet" => Ok(BitcoinNetwork::Mainnet),
            "testnet" => Ok(BitcoinNetwork::Testnet),
            other => Err(format!(
                "NETWORK must be 'mainnet' or 'testnet', got {other:?}"
            )),
        }
    }
}

/// Transport policy for the Stratum connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsPolicy {
    /// The bundled cpuminer engine speaks plaintext Stratum only (see
    /// SECURITY.md). This is the only policy this MVP can actually honor.
    PlaintextAllowed,
    Required,
}

impl TlsPolicy {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "plaintext" => Ok(TlsPolicy::PlaintextAllowed),
            "required" => Ok(TlsPolicy::Required),
            other => Err(format!(
                "TLS_POLICY must be 'plaintext' or 'required', got {other:?}"
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    // Validated at load time; consumed once MODE=shared ships (currently rejected above).
    #[allow(dead_code)]
    pub mode: MiningMode,
    // Validated at load time; consumed by payout address validation (US-008).
    #[allow(dead_code)]
    pub network: BitcoinNetwork,
    pub payout_address: String,
    pub pool_username: String,
    // Folded into `pool_username` at load time; kept for display/logging use.
    #[allow(dead_code)]
    pub worker_name: String,
    // Reserved for shared-pool authentication once MODE=shared ships.
    #[allow(dead_code)]
    pub secret_file: Option<String>,
    pub pool_url: String,
    // Validated at load time; consumed once TLS_POLICY=required ships.
    #[allow(dead_code)]
    pub tls_policy: TlsPolicy,
    pub power: u8,
    pub port: u16,
    pub miner_bin: String,
    pub miner_args: Option<String>,
    // Read in phase 2 (dashboard login screen)
    #[allow(dead_code)]
    pub dashboard_password: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        Self::from_vars(|k| std::env::var(k).ok())
    }

    fn from_vars<F: Fn(&str) -> Option<String>>(get: F) -> Result<Self, String> {
        let mode = match get("MODE") {
            None => MiningMode::Solo,
            Some(raw) => MiningMode::parse(&raw)?,
        };
        if mode == MiningMode::Shared {
            return Err(
                "MODE=shared is not implemented in this MVP; only solo mining against the \
                 fixed pool endpoint is supported. Set MODE=solo or omit it."
                    .into(),
            );
        }

        let network = match get("NETWORK") {
            None => BitcoinNetwork::Mainnet,
            Some(raw) => BitcoinNetwork::parse(&raw)?,
        };

        let tls_policy = match get("TLS_POLICY") {
            None => TlsPolicy::PlaintextAllowed,
            Some(raw) => TlsPolicy::parse(&raw)?,
        };
        if tls_policy == TlsPolicy::Required {
            return Err(
                "TLS_POLICY=required is not supported: the bundled cpuminer engine has no TLS \
                 support in this MVP (see SECURITY.md). Use TLS_POLICY=plaintext or omit it."
                    .into(),
            );
        }

        let payout_address = get("WALLET")
            .map(|w| w.trim().to_string())
            .filter(|w| !w.is_empty())
            .ok_or("WALLET is required (your BTC payout address)")?;
        if payout_address.len() < 26 {
            return Err(format!(
                "WALLET looks invalid (too short): {payout_address:?}"
            ));
        }

        let worker_name = get("WORKER_NAME").unwrap_or_else(|| "miner".into());

        let pool_username = match get("POOL_USERNAME") {
            Some(raw) => {
                let trimmed = raw.trim().to_string();
                if trimmed.is_empty() {
                    return Err("POOL_USERNAME, when set, cannot be empty".into());
                }
                trimmed
            }
            None if payout_address.contains('.') => payout_address.clone(),
            None => format!("{payout_address}.{worker_name}"),
        };

        let secret_file = match get("SECRET_FILE") {
            Some(raw) => {
                let path = raw.trim().to_string();
                if !Path::new(&path).is_file() {
                    return Err(format!(
                        "SECRET_FILE does not point to a readable file: {path:?}"
                    ));
                }
                Some(path)
            }
            None => None,
        };

        let power = match get("POWER") {
            None => 50,
            Some(raw) => raw
                .trim()
                .parse::<u8>()
                .ok()
                .filter(|p| (1..=100).contains(p))
                .ok_or(format!(
                    "POWER must be an integer from 1 to 100, got {raw:?}"
                ))?,
        };

        let port = match get("PORT") {
            None => 3500,
            Some(raw) => raw
                .trim()
                .parse::<u16>()
                .map_err(|_| format!("PORT must be a number, got {raw:?}"))?,
        };

        Ok(Config {
            mode,
            network,
            payout_address,
            pool_username,
            worker_name,
            secret_file,
            pool_url: get("POOL_URL").unwrap_or_else(|| DEFAULT_POOL_URL.into()),
            tls_policy,
            power,
            port,
            miner_bin: get("MINER_BIN").unwrap_or_else(|| "/usr/local/bin/minerd".into()),
            miner_args: get("MINER_ARGS").filter(|a| !a.trim().is_empty()),
            dashboard_password: get("DASHBOARD_PASSWORD").filter(|p| !p.is_empty()),
        })
    }

    /// Miner threads for a given core count, honoring POWER%. Never less than 1.
    pub fn threads(&self, cores: usize) -> usize {
        max(1, cores * self.power as usize / 100)
    }

    /// Arguments for the miner process. With MINER_ARGS set, the engine is
    /// fully pluggable (GPU miners, other algos): each token has {POOL},
    /// {USER} and {THREADS} substituted. Otherwise, defaults to cpuminer
    /// sha256d flags.
    pub fn miner_command_args(&self, threads: usize) -> Vec<String> {
        match &self.miner_args {
            Some(raw) => raw
                .split_whitespace()
                .map(|token| {
                    token
                        .replace("{POOL}", &self.pool_url)
                        .replace("{USER}", &self.pool_username)
                        .replace("{THREADS}", &threads.to_string())
                })
                .collect(),
            None => vec![
                "-a".into(),
                "sha256d".into(),
                "-o".into(),
                self.pool_url.clone(),
                "-u".into(),
                self.pool_username.clone(),
                "-p".into(),
                "x".into(),
                "-t".into(),
                threads.to_string(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn cfg(vars: &[(&str, &str)]) -> Result<Config, String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_vars(|k| map.get(k).cloned())
    }

    const WALLET: (&str, &str) = ("WALLET", "bc1qexamplewalletaddress0000000000");

    #[test]
    fn wallet_is_required() {
        assert!(cfg(&[]).is_err());
        assert!(cfg(&[("WALLET", "   ")]).is_err());
    }

    #[test]
    fn wallet_is_trimmed() {
        let c = cfg(&[("WALLET", "  bc1qexamplewalletaddress0000000000  ")]).unwrap();
        assert_eq!(c.payout_address, "bc1qexamplewalletaddress0000000000");
    }

    #[test]
    fn short_wallet_is_rejected() {
        assert!(cfg(&[("WALLET", "bc1qshort")]).is_err());
    }

    #[test]
    fn power_defaults_to_50() {
        assert_eq!(cfg(&[WALLET]).unwrap().power, 50);
    }

    #[test]
    fn power_out_of_range_is_rejected() {
        for bad in ["0", "101", "150", "abc", "-5", ""] {
            assert!(
                cfg(&[WALLET, ("POWER", bad)]).is_err(),
                "POWER={bad:?} should fail"
            );
        }
    }

    #[test]
    fn power_bounds_are_accepted() {
        assert_eq!(cfg(&[WALLET, ("POWER", "1")]).unwrap().power, 1);
        assert_eq!(cfg(&[WALLET, ("POWER", "100")]).unwrap().power, 100);
    }

    #[test]
    fn threads_honor_power_and_never_go_below_one() {
        let c = cfg(&[WALLET, ("POWER", "50")]).unwrap();
        assert_eq!(c.threads(4), 2);
        assert_eq!(c.threads(1), 1);

        let c = cfg(&[WALLET, ("POWER", "100")]).unwrap();
        assert_eq!(c.threads(8), 8);

        let c = cfg(&[WALLET, ("POWER", "1")]).unwrap();
        assert_eq!(c.threads(64), 1);
    }

    #[test]
    fn pool_username_appends_worker_name() {
        let c = cfg(&[WALLET, ("WORKER_NAME", "vps1")]).unwrap();
        assert_eq!(c.pool_username, "bc1qexamplewalletaddress0000000000.vps1");
    }

    #[test]
    fn pool_username_keeps_wallet_with_embedded_worker() {
        let c = cfg(&[("WALLET", "bc1qexamplewalletaddress0000000000.rig")]).unwrap();
        assert_eq!(c.pool_username, "bc1qexamplewalletaddress0000000000.rig");
    }

    #[test]
    fn pool_username_override_is_used_verbatim() {
        let c = cfg(&[WALLET, ("POOL_USERNAME", "custom.user")]).unwrap();
        assert_eq!(c.pool_username, "custom.user");
    }

    #[test]
    fn empty_pool_username_override_is_rejected() {
        assert!(cfg(&[WALLET, ("POOL_USERNAME", "   ")]).is_err());
    }

    #[test]
    fn mode_defaults_to_solo() {
        let c = cfg(&[WALLET]).unwrap();
        assert_eq!(c.mode, MiningMode::Solo);
    }

    #[test]
    fn mode_shared_is_explicitly_rejected() {
        let err = cfg(&[WALLET, ("MODE", "shared")]).unwrap_err();
        assert!(err.contains("not implemented"), "unexpected error: {err}");
    }

    #[test]
    fn mode_rejects_unknown_value() {
        assert!(cfg(&[WALLET, ("MODE", "pooled")]).is_err());
    }

    #[test]
    fn network_defaults_to_mainnet() {
        let c = cfg(&[WALLET]).unwrap();
        assert_eq!(c.network, BitcoinNetwork::Mainnet);
    }

    #[test]
    fn network_accepts_testnet() {
        let c = cfg(&[WALLET, ("NETWORK", "testnet")]).unwrap();
        assert_eq!(c.network, BitcoinNetwork::Testnet);
    }

    #[test]
    fn network_rejects_unknown_value() {
        assert!(cfg(&[WALLET, ("NETWORK", "regtest")]).is_err());
    }

    #[test]
    fn tls_policy_defaults_to_plaintext_allowed() {
        let c = cfg(&[WALLET]).unwrap();
        assert_eq!(c.tls_policy, TlsPolicy::PlaintextAllowed);
    }

    #[test]
    fn tls_policy_required_is_explicitly_rejected() {
        let err = cfg(&[WALLET, ("TLS_POLICY", "required")]).unwrap_err();
        assert!(err.contains("not supported"), "unexpected error: {err}");
    }

    #[test]
    fn tls_policy_rejects_unknown_value() {
        assert!(cfg(&[WALLET, ("TLS_POLICY", "opportunistic")]).is_err());
    }

    #[test]
    fn secret_file_missing_path_is_rejected() {
        assert!(cfg(&[WALLET, ("SECRET_FILE", "/nonexistent/secret")]).is_err());
    }

    #[test]
    fn secret_file_existing_path_is_accepted() {
        let path = std::env::temp_dir().join("miner-config-test-secret-file");
        std::fs::write(&path, "sekrit\n").unwrap();
        let c = cfg(&[WALLET, ("SECRET_FILE", path.to_str().unwrap())]).unwrap();
        assert_eq!(c.secret_file.as_deref(), path.to_str());
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn secret_file_defaults_to_none() {
        let c = cfg(&[WALLET]).unwrap();
        assert!(c.secret_file.is_none());
    }

    #[test]
    fn default_miner_args_target_sha256d() {
        let c = cfg(&[WALLET]).unwrap();
        assert_eq!(
            c.miner_command_args(2),
            vec![
                "-a",
                "sha256d",
                "-o",
                "stratum+tcp://public-pool.io:21496",
                "-u",
                "bc1qexamplewalletaddress0000000000.miner",
                "-p",
                "x",
                "-t",
                "2",
            ]
        );
    }

    #[test]
    fn custom_miner_args_substitute_placeholders() {
        let c = cfg(&[
            WALLET,
            (
                "MINER_ARGS",
                "--url {POOL} --user {USER} --threads {THREADS} --gpu 0",
            ),
        ])
        .unwrap();
        assert_eq!(
            c.miner_command_args(4),
            vec![
                "--url",
                "stratum+tcp://public-pool.io:21496",
                "--user",
                "bc1qexamplewalletaddress0000000000.miner",
                "--threads",
                "4",
                "--gpu",
                "0",
            ]
        );
    }

    #[test]
    fn custom_miner_args_without_placeholders_pass_verbatim() {
        let c = cfg(&[WALLET, ("MINER_ARGS", "--benchmark")]).unwrap();
        assert_eq!(c.miner_command_args(8), vec!["--benchmark"]);
    }

    #[test]
    fn blank_miner_args_fall_back_to_default() {
        let c = cfg(&[WALLET, ("MINER_ARGS", "   ")]).unwrap();
        assert_eq!(c.miner_command_args(1)[..2], ["-a", "sha256d"]);
    }

    #[test]
    fn defaults_point_to_public_pool() {
        let c = cfg(&[WALLET]).unwrap();
        assert_eq!(c.pool_url, "stratum+tcp://public-pool.io:21496");
        assert_eq!(c.port, 3500);
        assert!(c.dashboard_password.is_none());
    }
}
