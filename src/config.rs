use std::cmp::max;

#[derive(Debug, Clone)]
pub struct Config {
    pub wallet: String,
    pub pool_url: String,
    pub worker_name: String,
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
        let wallet = get("WALLET")
            .map(|w| w.trim().to_string())
            .filter(|w| !w.is_empty())
            .ok_or("WALLET is required (your BTC address)")?;
        if wallet.len() < 26 {
            return Err(format!("WALLET looks invalid (too short): {wallet:?}"));
        }

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
            wallet,
            pool_url: get("POOL_URL")
                .unwrap_or_else(|| "stratum+tcp://public-pool.io:21496".into()),
            worker_name: get("WORKER_NAME").unwrap_or_else(|| "miner".into()),
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

    /// Stratum username: `wallet.worker`, unless the wallet already embeds a worker name.
    pub fn stratum_user(&self) -> String {
        if self.wallet.contains('.') {
            self.wallet.clone()
        } else {
            format!("{}.{}", self.wallet, self.worker_name)
        }
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
                        .replace("{USER}", &self.stratum_user())
                        .replace("{THREADS}", &threads.to_string())
                })
                .collect(),
            None => vec![
                "-a".into(),
                "sha256d".into(),
                "-o".into(),
                self.pool_url.clone(),
                "-u".into(),
                self.stratum_user(),
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
        assert_eq!(c.wallet, "bc1qexamplewalletaddress0000000000");
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
    fn stratum_user_appends_worker_name() {
        let c = cfg(&[WALLET, ("WORKER_NAME", "vps1")]).unwrap();
        assert_eq!(c.stratum_user(), "bc1qexamplewalletaddress0000000000.vps1");
    }

    #[test]
    fn stratum_user_keeps_wallet_with_embedded_worker() {
        let c = cfg(&[("WALLET", "bc1qexamplewalletaddress0000000000.rig")]).unwrap();
        assert_eq!(c.stratum_user(), "bc1qexamplewalletaddress0000000000.rig");
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
