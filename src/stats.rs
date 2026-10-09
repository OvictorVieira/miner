use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use tokio::sync::RwLock;

const CACHE_TTL: Duration = Duration::from_secs(30);
const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
const NEXT_HALVING_BLOCK: u64 = 1_050_000;

pub const POOL_API: &str = "https://public-pool.io:40557/api";
pub const MEMPOOL_API: &str = "https://mempool.space/api";

fn pool_client_url(wallet: &str) -> String {
    let mut url = reqwest::Url::parse(POOL_API).expect("POOL_API must be a valid fixed URL");
    url.path_segments_mut()
        .expect("POOL_API must be a hierarchical URL")
        .extend(["client", wallet]);
    url.into()
}

/// Real miner stats from the Public Pool client API. All zeros until the
/// pool has seen shares from this wallet.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct PoolStats {
    pub hashrate_10m: f64,
    pub hashrate_1h: f64,
    pub best_difficulty: f64,
    pub workers: u64,
    pub accepted_shares: u64,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct NetworkStats {
    pub btc_usd: f64,
    pub block_height: u64,
    pub difficulty: f64,
    pub last_block_pool: String,
    pub halving_blocks_left: u64,
    pub halving_days_left: u64,
}

pub fn parse_pool_stats(v: &Value) -> PoolStats {
    let acc = &v["accounting"];
    // The pool reports a best difficulty in up to three places depending on
    // session state; take the max so it never shows 0 while shares exist.
    let workers_best = v["workers"]
        .as_array()
        .map(|ws| {
            ws.iter()
                .filter_map(|w| w["bestDifficulty"].as_f64())
                .fold(0.0, f64::max)
        })
        .unwrap_or(0.0);
    let best_difficulty = v["bestDifficulty"]
        .as_f64()
        .unwrap_or(0.0)
        .max(acc["bestSubmissionDifficulty"].as_f64().unwrap_or(0.0))
        .max(workers_best);

    PoolStats {
        hashrate_10m: acc["hashRateLast10Minutes"].as_f64().unwrap_or(0.0),
        hashrate_1h: acc["hashRateLastHour"].as_f64().unwrap_or(0.0),
        best_difficulty,
        workers: v["workersCount"].as_u64().unwrap_or(0),
        accepted_shares: acc["totalAcceptedShares"].as_u64().unwrap_or(0),
    }
}

pub fn parse_network_stats(prices: &Value, height: u64, blocks: &Value) -> NetworkStats {
    let tip = &blocks[0];
    let blocks_left = NEXT_HALVING_BLOCK.saturating_sub(height);
    NetworkStats {
        btc_usd: prices["USD"].as_f64().unwrap_or(0.0),
        block_height: height,
        difficulty: tip["difficulty"].as_f64().unwrap_or(0.0),
        last_block_pool: tip["extras"]["pool"]["name"]
            .as_str()
            .or_else(|| tip["pool"]["name"].as_str())
            .unwrap_or("unknown")
            .to_string(),
        halving_blocks_left: blocks_left,
        // ~10 minutes per block
        halving_days_left: blocks_left * 10 / 1440,
    }
}

#[derive(Default)]
struct Cached {
    pool: PoolStats,
    network: NetworkStats,
    fetched_at: Option<Instant>,
}

/// TTL cache in front of the external APIs so the dashboard can poll
/// aggressively without hammering them.
pub struct StatsCache {
    client: reqwest::Client,
    inner: RwLock<Cached>,
}

impl StatsCache {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .user_agent(concat!("community-miner/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("failed to build http client");
        Self {
            client,
            inner: RwLock::new(Cached::default()),
        }
    }

    pub async fn get(&self, wallet: &str) -> (PoolStats, NetworkStats) {
        {
            let cached = self.inner.read().await;
            if cached.fetched_at.is_some_and(|t| t.elapsed() < CACHE_TTL) {
                return (cached.pool.clone(), cached.network.clone());
            }
        }

        let (pool, network) = tokio::join!(self.fetch_pool(wallet), self.fetch_network());

        let mut cached = self.inner.write().await;
        // Keep the previous values when a fetch fails
        if let Some(p) = pool {
            cached.pool = p;
        }
        if let Some(n) = network {
            cached.network = n;
        }
        cached.fetched_at = Some(Instant::now());
        (cached.pool.clone(), cached.network.clone())
    }

    async fn fetch_json(&self, url: String) -> Option<Value> {
        match self.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => resp.json().await.ok(),
            Ok(resp) => {
                tracing::warn!(url, status = %resp.status(), "stats fetch failed");
                None
            }
            Err(e) => {
                tracing::warn!(url, error = %e, "stats fetch failed");
                None
            }
        }
    }

    async fn fetch_pool(&self, wallet: &str) -> Option<PoolStats> {
        let v = self.fetch_json(pool_client_url(wallet)).await?;
        Some(parse_pool_stats(&v))
    }

    async fn fetch_network(&self) -> Option<NetworkStats> {
        let (prices, height, blocks) = tokio::join!(
            self.fetch_json(format!("{MEMPOOL_API}/v1/prices")),
            self.fetch_text(format!("{MEMPOOL_API}/blocks/tip/height")),
            self.fetch_json(format!("{MEMPOOL_API}/v1/blocks")),
        );
        let height: u64 = height?.trim().parse().ok()?;
        Some(parse_network_stats(&prices?, height, &blocks?))
    }

    async fn fetch_text(&self, url: String) -> Option<String> {
        match self.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => resp.text().await.ok(),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Trimmed real response from public-pool.io:40557/api/client/{address}
    fn pool_fixture() -> Value {
        json!({
            "bestDifficulty": 1423190.5,
            "workersCount": 2,
            "accounting": {
                "totalAcceptedShares": 1784,
                "hashRateLast10Minutes": 9_300_000.0,
                "hashRateLastHour": 8_100_000.0
            },
            "workers": []
        })
    }

    #[test]
    fn parses_pool_stats_from_real_shape() {
        let s = parse_pool_stats(&pool_fixture());
        assert_eq!(
            s,
            PoolStats {
                hashrate_10m: 9_300_000.0,
                hashrate_1h: 8_100_000.0,
                best_difficulty: 1_423_190.5,
                workers: 2,
                accepted_shares: 1784,
            }
        );
    }

    #[test]
    fn pool_stats_default_to_zero_on_missing_fields() {
        assert_eq!(parse_pool_stats(&json!({})), PoolStats::default());
    }

    #[test]
    fn pool_client_url_percent_encodes_the_wallet_path_segment() {
        assert_eq!(
            pool_client_url("wallet/../other?admin=true#fragment with space"),
            "https://public-pool.io:40557/api/client/wallet%2F..%2Fother%3Fadmin=true%23fragment%20with%20space"
        );
        assert_eq!(
            pool_client_url("bc1qvalidatedwallet"),
            "https://public-pool.io:40557/api/client/bc1qvalidatedwallet"
        );
    }

    #[test]
    fn best_difficulty_takes_the_max_across_all_sources() {
        // Live sessions sometimes report 0 at the top level while the real
        // best sits in accounting or per-worker data
        let v = json!({
            "bestDifficulty": 0,
            "workersCount": 1,
            "accounting": {"totalAcceptedShares": 117, "bestSubmissionDifficulty": 8500.0},
            "workers": [{"name": "homelab", "bestDifficulty": 12000.5}]
        });
        assert_eq!(parse_pool_stats(&v).best_difficulty, 12000.5);
    }

    #[test]
    fn parses_network_stats_from_real_shapes() {
        let prices = json!({"time": 1783004712, "USD": 61539});
        let blocks = json!([{
            "height": 956368,
            "difficulty": 133869853540305.4,
            "extras": {"pool": {"id": 111, "name": "Foundry USA"}}
        }]);
        let n = parse_network_stats(&prices, 956_368, &blocks);
        assert_eq!(n.btc_usd, 61539.0);
        assert_eq!(n.block_height, 956_368);
        assert_eq!(n.last_block_pool, "Foundry USA");
        assert_eq!(n.halving_blocks_left, 1_050_000 - 956_368);
        assert_eq!(n.halving_days_left, (1_050_000 - 956_368) * 10 / 1440);
    }

    #[test]
    fn network_stats_survive_missing_pool_name() {
        let n = parse_network_stats(&json!({}), 1_060_000, &json!([{}]));
        assert_eq!(n.last_block_pool, "unknown");
        // Past the halving block: saturates at zero instead of underflowing
        assert_eq!(n.halving_blocks_left, 0);
    }
}
