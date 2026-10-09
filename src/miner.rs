use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::sync::{watch, RwLock};

use crate::config::Config;

#[derive(Default)]
pub struct MinerStatus {
    pub running: bool,
    pub pid: Option<u32>,
    pub restarts: u32,
    pub started_at: Option<Instant>,
    pub last_error: Option<String>,
}

pub type SharedStatus = Arc<RwLock<MinerStatus>>;

const INITIAL_BACKOFF: Duration = Duration::from_secs(2);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
// A run longer than this is considered healthy, so the next failure restarts fast.
const HEALTHY_RUN: Duration = Duration::from_secs(60);

pub fn spawn_supervisor(
    cfg: Config,
    status: SharedStatus,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = INITIAL_BACKOFF;
        loop {
            if *shutdown.borrow() {
                return;
            }

            let cores = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);
            let threads = cfg.threads(cores);

            // minerd logs shares to stderr; inheriting sends them to `docker logs`.
            // Arguments are passed as an argv array via `.args(...)` — never a
            // shell string — so no token can be shell-expanded.
            let spawned = Command::new(cfg.engine.binary())
                .args(cfg.miner_command_args(threads))
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .kill_on_drop(true)
                .spawn();

            match spawned {
                Ok(mut child) => {
                    let started = Instant::now();
                    {
                        let mut s = status.write().await;
                        s.running = true;
                        s.pid = child.id();
                        s.started_at = Some(started);
                        s.last_error = None;
                    }
                    tracing::info!(
                        pid = child.id(),
                        threads,
                        pool = %cfg.pool_url,
                        "miner started"
                    );

                    tokio::select! {
                        result = child.wait() => {
                            {
                                let mut s = status.write().await;
                                s.running = false;
                                s.pid = None;
                                s.restarts += 1;
                                s.last_error = Some(match &result {
                                    Ok(st) => format!("miner exited: {st}"),
                                    Err(e) => format!("miner wait failed: {e}"),
                                });
                            }
                            if started.elapsed() > HEALTHY_RUN {
                                backoff = INITIAL_BACKOFF;
                            }
                            tracing::warn!(?result, "miner exited, restarting in {backoff:?}");
                        }
                        _ = shutdown.changed() => {
                            tracing::info!("shutting down miner");
                            let _ = child.kill().await;
                            let mut s = status.write().await;
                            s.running = false;
                            s.pid = None;
                            return;
                        }
                    }
                }
                Err(e) => {
                    let bin = cfg.engine.binary();
                    {
                        let mut s = status.write().await;
                        s.running = false;
                        s.last_error = Some(format!("failed to start {bin}: {e}"));
                    }
                    tracing::error!(bin = %bin, error = %e, "failed to start miner, retrying in {backoff:?}");
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(backoff) => {}
                _ = shutdown.changed() => return,
            }
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    })
}
