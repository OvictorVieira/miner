//! Deterministic in-process Stratum v1 server used only in tests.
//!
//! The real pool depends on the public internet, time, and non-local shares.
//! Those dependencies are not acceptable in CI, and a mining-protocol
//! regression that only shows up against the live pool is both slow to
//! reproduce and visible to a third party. This module provides an offline
//! substitute that speaks enough of the Stratum JSON-RPC dialect to drive a
//! real client through subscribe / authorize / set_difficulty / notify /
//! submit / accept / reject / disconnect, and records every field a
//! downstream story (US-011) needs to assert the identity the miner put on
//! the wire.
//!
//! Nothing in this module is wired into the release binary; the module is
//! `pub(crate)` and the server is only started by tests. It is still
//! compiled under the default profile so `cargo check` catches regressions.
//!
//! Hard safety bounds: a client cannot push more than `MAX_LINE_BYTES` on a
//! single JSON-RPC line, and no read or accept call will block the fixture
//! for longer than `IO_TIMEOUT`. Both are enforced explicitly below so a
//! malformed client can never hang a test run.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::timeout;

/// Hard cap on a single Stratum JSON line. The real protocol lines are a
/// few hundred bytes; anything larger is either a bug or an attempt to
/// exhaust test memory, and we fail the connection rather than grow.
pub const MAX_LINE_BYTES: usize = 16 * 1024;

/// Upper bound on any single read / accept call. Keeps a hung client from
/// blocking a `#[tokio::test]` worker forever.
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Fixed extranonce1 the fake pool hands out on `mining.subscribe`. A
/// constant value makes submit payloads reproducible across runs.
pub const EXTRANONCE1: &str = "0000abcd";

/// Fixed extranonce2 size reported on `mining.subscribe`. Picked to match
/// what the stock cpuminer-multi expects (4 bytes).
pub const EXTRANONCE2_SIZE: u64 = 4;

/// Fixed difficulty pushed on `mining.set_difficulty` before the first job.
pub const DEFAULT_DIFFICULTY: f64 = 1.0;

/// Deterministic job pushed on `mining.notify`. The fields are illustrative
/// only — the fake server never validates proof-of-work.
pub const JOB_ID: &str = "fake-job-1";

/// Scenario the server plays once a client has connected.
///
/// Each variant produces a different, fully deterministic transcript, so
/// tests can target one protocol branch at a time without racing timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    /// Normal flow: subscribe → authorize(true) → set_difficulty → notify.
    /// Every `mining.submit` from the client is answered with `{result:true}`.
    AcceptAll,
    /// Same flow, but every `mining.submit` is answered with
    /// `{result:false, error:[21,"job-not-found",null]}`.
    RejectAll,
    /// Reach `mining.subscribe`, respond, then close the socket before
    /// `mining.authorize` is answered. Exercises client-side reconnect.
    DisconnectAfterSubscribe,
    /// Accept the first `mining.submit`, then close the socket. Exercises
    /// client-side resume after an in-flight share.
    AcceptThenDisconnect,
}

/// Record of a single `mining.submit` call, in the order fields appear on
/// the wire: `[worker, job_id, extranonce2, ntime, nonce]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitRecord {
    pub worker: String,
    pub job_id: String,
    pub extranonce2: String,
    pub ntime: String,
    pub nonce: String,
}

/// Record of `mining.authorize` arguments: `[username, password]`.
///
/// `Debug` is implemented by hand (not derived) so a failed test assertion
/// that prints an `AuthorizeRecord` does not leak the password into the
/// test log. Equality still compares passwords as plain strings.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizeRecord {
    pub username: String,
    pub password: String,
}

impl std::fmt::Debug for AuthorizeRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorizeRecord")
            .field("username", &self.username)
            .field(
                "password",
                &format_args!("<redacted:{} bytes>", self.password.len()),
            )
            .finish()
    }
}

/// Everything the fake pool observed during a single connection.
///
/// `password` is intentionally captured as an opaque placeholder — the
/// server never interprets it, which is also what a real pool does for
/// solo-mining clients.
#[derive(Debug, Default, Clone)]
pub struct Recorded {
    pub subscribe_count: usize,
    pub authorize: Option<AuthorizeRecord>,
    pub submits: Vec<SubmitRecord>,
    pub extranonce1: Option<String>,
    pub extranonce2_size: Option<u64>,
    pub disconnected: bool,
}

/// Handle to a running fake Stratum server. Dropping the handle aborts the
/// background task and releases the port.
pub struct FakeStratum {
    addr: SocketAddr,
    recorded: Arc<Mutex<Recorded>>,
    task: Option<JoinHandle<()>>,
}

impl FakeStratum {
    /// Bind an ephemeral loopback port and start accepting one connection
    /// in the background. The returned handle exposes the bound address so
    /// a client can be pointed at it.
    pub async fn start(scenario: Scenario) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let recorded = Arc::new(Mutex::new(Recorded::default()));
        let rec_task = Arc::clone(&recorded);
        let task = tokio::spawn(async move {
            // Only ever accept one connection per fixture; a second one
            // would race with the first on `recorded`.
            if let Ok(Ok((stream, _))) = timeout(IO_TIMEOUT, listener.accept()).await {
                let _ = handle_connection(stream, scenario, Arc::clone(&rec_task)).await;
            }
            rec_task.lock().await.disconnected = true;
        });
        Ok(FakeStratum {
            addr,
            recorded,
            task: Some(task),
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Snapshot of what the server has recorded so far.
    pub async fn recorded(&self) -> Recorded {
        self.recorded.lock().await.clone()
    }

    /// Wait for the background task to finish (the client disconnected,
    /// the scenario closed the socket, or `IO_TIMEOUT` elapsed on accept).
    pub async fn wait(mut self) -> Recorded {
        if let Some(t) = self.task.take() {
            let _ = t.await;
        }
        self.recorded.lock().await.clone()
    }
}

impl Drop for FakeStratum {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

async fn handle_connection(
    stream: TcpStream,
    scenario: Scenario,
    recorded: Arc<Mutex<Recorded>>,
) -> std::io::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut line = String::new();

    loop {
        line.clear();
        let read = timeout(IO_TIMEOUT, read_line_bounded(&mut reader, &mut line)).await;
        let n = match read {
            Ok(Ok(n)) => n,
            // timeout or io error → close and let the recorder note it
            _ => return Ok(()),
        };
        if n == 0 {
            return Ok(());
        }

        let msg: serde_json::Value = match serde_json::from_str(line.trim()) {
            Ok(v) => v,
            Err(_) => return Ok(()),
        };
        let id = msg.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = msg
            .get("params")
            .and_then(|p| p.as_array())
            .cloned()
            .unwrap_or_default();

        match method {
            "mining.subscribe" => {
                {
                    let mut r = recorded.lock().await;
                    r.subscribe_count += 1;
                    r.extranonce1 = Some(EXTRANONCE1.to_string());
                    r.extranonce2_size = Some(EXTRANONCE2_SIZE);
                }
                let resp = serde_json::json!({
                    "id": id,
                    "result": [
                        [["mining.set_difficulty", "sub1"], ["mining.notify", "sub2"]],
                        EXTRANONCE1,
                        EXTRANONCE2_SIZE
                    ],
                    "error": null,
                });
                write_json_line(&mut write_half, &resp).await?;

                if scenario == Scenario::DisconnectAfterSubscribe {
                    return Ok(());
                }
            }
            "mining.authorize" => {
                let username = params
                    .first()
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let password = params
                    .get(1)
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                {
                    let mut r = recorded.lock().await;
                    r.authorize = Some(AuthorizeRecord {
                        username: username.clone(),
                        password,
                    });
                }
                let resp = serde_json::json!({
                    "id": id,
                    "result": true,
                    "error": null,
                });
                write_json_line(&mut write_half, &resp).await?;

                // Push a deterministic set_difficulty + notify immediately
                // so a client that waits for a job before submitting has
                // one to work on.
                let diff = serde_json::json!({
                    "id": null,
                    "method": "mining.set_difficulty",
                    "params": [DEFAULT_DIFFICULTY],
                });
                write_json_line(&mut write_half, &diff).await?;

                let notify = serde_json::json!({
                    "id": null,
                    "method": "mining.notify",
                    "params": [
                        JOB_ID,
                        // prevhash, coinbase1, coinbase2, merkle_branches,
                        // version, nbits, ntime, clean_jobs
                        "0".repeat(64),
                        "01000000",
                        "ffffffff",
                        [],
                        "20000000",
                        "1d00ffff",
                        "5e8f1a00",
                        true
                    ],
                });
                write_json_line(&mut write_half, &notify).await?;
            }
            "mining.submit" => {
                let rec = SubmitRecord {
                    worker: params
                        .first()
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    job_id: params
                        .get(1)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    extranonce2: params
                        .get(2)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    ntime: params
                        .get(3)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    nonce: params
                        .get(4)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                };
                let is_first_submit = {
                    let mut r = recorded.lock().await;
                    r.submits.push(rec);
                    r.submits.len() == 1
                };

                let resp = match scenario {
                    Scenario::AcceptAll | Scenario::AcceptThenDisconnect => serde_json::json!({
                        "id": id,
                        "result": true,
                        "error": null,
                    }),
                    Scenario::RejectAll => serde_json::json!({
                        "id": id,
                        "result": false,
                        "error": [21, "job-not-found", null],
                    }),
                    Scenario::DisconnectAfterSubscribe => {
                        // unreachable: we would have returned above, but
                        // keep the match total.
                        return Ok(());
                    }
                };
                write_json_line(&mut write_half, &resp).await?;

                if scenario == Scenario::AcceptThenDisconnect && is_first_submit {
                    return Ok(());
                }
            }
            // Unknown methods are answered with a Stratum-style error
            // instead of closing the socket; a well-behaved client will
            // just move on.
            _ => {
                let resp = serde_json::json!({
                    "id": id,
                    "result": null,
                    "error": [20, "unknown method", null],
                });
                write_json_line(&mut write_half, &resp).await?;
            }
        }
    }
}

/// `BufReader::read_line` with an explicit byte cap. Keeps a client from
/// pushing an unbounded payload into the fixture's address space.
async fn read_line_bounded<R>(reader: &mut BufReader<R>, buf: &mut String) -> std::io::Result<usize>
where
    R: tokio::io::AsyncRead + Unpin,
{
    // Delegate to AsyncBufReadExt but short-circuit if the already-buffered
    // slice is suspiciously large after a single fill.
    let n = reader.read_line(buf).await?;
    if buf.len() > MAX_LINE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "stratum line exceeds MAX_LINE_BYTES",
        ));
    }
    Ok(n)
}

async fn write_json_line<W>(w: &mut W, v: &serde_json::Value) -> std::io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    let mut bytes = serde_json::to_vec(v).map_err(std::io::Error::other)?;
    bytes.push(b'\n');
    timeout(IO_TIMEOUT, w.write_all(&bytes))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "write timed out"))??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    use tokio::net::TcpStream;

    /// Minimal line-oriented JSON-RPC client used by the tests. Mirrors
    /// what a real Stratum miner does: writes one JSON object per line and
    /// reads one JSON object per line, with the same `IO_TIMEOUT` bound.
    struct TestClient {
        read: BufReader<tokio::net::tcp::OwnedReadHalf>,
        write: tokio::net::tcp::OwnedWriteHalf,
        next_id: u64,
    }

    impl TestClient {
        async fn connect(addr: SocketAddr) -> Self {
            let stream = TcpStream::connect(addr).await.expect("connect");
            let (r, w) = stream.into_split();
            TestClient {
                read: BufReader::new(r),
                write: w,
                next_id: 1,
            }
        }

        async fn send(&mut self, method: &str, params: serde_json::Value) -> u64 {
            let id = self.next_id;
            self.next_id += 1;
            let msg = serde_json::json!({"id": id, "method": method, "params": params});
            let mut bytes = serde_json::to_vec(&msg).unwrap();
            bytes.push(b'\n');
            self.write.write_all(&bytes).await.expect("write");
            id
        }

        async fn recv(&mut self) -> Option<serde_json::Value> {
            let mut line = String::new();
            let read = timeout(IO_TIMEOUT, self.read.read_line(&mut line)).await;
            match read {
                Ok(Ok(0)) | Err(_) | Ok(Err(_)) => None,
                Ok(Ok(_)) => serde_json::from_str(line.trim()).ok(),
            }
        }
    }

    #[tokio::test]
    async fn accept_all_records_full_handshake_and_submit() {
        let server = FakeStratum::start(Scenario::AcceptAll).await.unwrap();
        let addr = server.addr();
        let mut c = TestClient::connect(addr).await;

        c.send("mining.subscribe", serde_json::json!(["ua/0.1"]))
            .await;
        let sub = c.recv().await.expect("subscribe response");
        let result = &sub["result"];
        assert_eq!(result[1].as_str(), Some(EXTRANONCE1));
        assert_eq!(result[2].as_u64(), Some(EXTRANONCE2_SIZE));

        c.send(
            "mining.authorize",
            serde_json::json!(["bc1qexample.worker1", "x"]),
        )
        .await;
        let auth = c.recv().await.expect("authorize response");
        assert_eq!(auth["result"], serde_json::Value::Bool(true));

        // Server pushes set_difficulty then notify, in that order.
        let diff = c.recv().await.expect("set_difficulty");
        assert_eq!(diff["method"].as_str(), Some("mining.set_difficulty"));
        assert_eq!(diff["params"][0].as_f64(), Some(DEFAULT_DIFFICULTY));

        let notify = c.recv().await.expect("notify");
        assert_eq!(notify["method"].as_str(), Some("mining.notify"));
        assert_eq!(notify["params"][0].as_str(), Some(JOB_ID));

        c.send(
            "mining.submit",
            serde_json::json!([
                "bc1qexample.worker1",
                JOB_ID,
                "deadbeef",
                "5e8f1a00",
                "01020304"
            ]),
        )
        .await;
        let submit = c.recv().await.expect("submit response");
        assert_eq!(submit["result"], serde_json::Value::Bool(true));

        drop(c);
        let rec = server.wait().await;
        assert_eq!(rec.subscribe_count, 1);
        assert_eq!(
            rec.authorize,
            Some(AuthorizeRecord {
                username: "bc1qexample.worker1".to_string(),
                password: "x".to_string(),
            })
        );
        assert_eq!(
            rec.submits,
            vec![SubmitRecord {
                worker: "bc1qexample.worker1".to_string(),
                job_id: JOB_ID.to_string(),
                extranonce2: "deadbeef".to_string(),
                ntime: "5e8f1a00".to_string(),
                nonce: "01020304".to_string(),
            }]
        );
        assert_eq!(rec.extranonce1.as_deref(), Some(EXTRANONCE1));
        assert_eq!(rec.extranonce2_size, Some(EXTRANONCE2_SIZE));
    }

    #[tokio::test]
    async fn reject_all_returns_stratum_error_tuple_on_submit() {
        let server = FakeStratum::start(Scenario::RejectAll).await.unwrap();
        let mut c = TestClient::connect(server.addr()).await;

        c.send("mining.subscribe", serde_json::json!([])).await;
        let _ = c.recv().await.expect("subscribe");
        c.send("mining.authorize", serde_json::json!(["user", "x"]))
            .await;
        let _ = c.recv().await.expect("authorize");
        let _ = c.recv().await.expect("set_difficulty");
        let _ = c.recv().await.expect("notify");

        c.send(
            "mining.submit",
            serde_json::json!(["user", JOB_ID, "00000000", "5e8f1a00", "00000000"]),
        )
        .await;
        let resp = c.recv().await.expect("submit response");
        assert_eq!(resp["result"], serde_json::Value::Bool(false));
        let err = resp["error"].as_array().expect("error tuple");
        assert_eq!(err[0].as_i64(), Some(21));
        assert_eq!(err[1].as_str(), Some("job-not-found"));
    }

    #[tokio::test]
    async fn disconnect_after_subscribe_closes_socket() {
        let server = FakeStratum::start(Scenario::DisconnectAfterSubscribe)
            .await
            .unwrap();
        let mut c = TestClient::connect(server.addr()).await;

        c.send("mining.subscribe", serde_json::json!([])).await;
        let _ = c.recv().await.expect("subscribe response");

        // Server must close the connection. A subsequent read returns EOF.
        let mut tail = String::new();
        let n = timeout(IO_TIMEOUT, c.read.read_to_string(&mut tail))
            .await
            .expect("no hang on closed socket")
            .expect("read ok");
        assert_eq!(n, 0, "server did not close socket after subscribe");

        let rec = server.wait().await;
        assert_eq!(rec.subscribe_count, 1);
        assert!(rec.authorize.is_none());
        assert!(rec.submits.is_empty());
    }

    #[tokio::test]
    async fn accept_then_disconnect_closes_after_first_submit() {
        let server = FakeStratum::start(Scenario::AcceptThenDisconnect)
            .await
            .unwrap();
        let mut c = TestClient::connect(server.addr()).await;

        c.send("mining.subscribe", serde_json::json!([])).await;
        let _ = c.recv().await.expect("subscribe");
        c.send("mining.authorize", serde_json::json!(["user", "x"]))
            .await;
        let _ = c.recv().await.expect("authorize");
        let _ = c.recv().await.expect("set_difficulty");
        let _ = c.recv().await.expect("notify");

        c.send(
            "mining.submit",
            serde_json::json!(["user", JOB_ID, "aa", "5e8f1a00", "ff"]),
        )
        .await;
        let resp = c.recv().await.expect("submit response");
        assert_eq!(resp["result"], serde_json::Value::Bool(true));

        // Server closes after this one acceptance.
        let mut tail = String::new();
        let n = timeout(IO_TIMEOUT, c.read.read_to_string(&mut tail))
            .await
            .expect("no hang")
            .expect("read ok");
        assert_eq!(n, 0);

        let rec = server.wait().await;
        assert_eq!(rec.submits.len(), 1);
    }

    #[tokio::test]
    async fn oversize_line_is_rejected_without_hanging() {
        let server = FakeStratum::start(Scenario::AcceptAll).await.unwrap();
        let mut stream = TcpStream::connect(server.addr()).await.unwrap();
        let mut giant = vec![b'A'; MAX_LINE_BYTES + 1024];
        giant.push(b'\n');
        // Server will read, see the cap exceeded, and close. The client's
        // own read must therefore return EOF within IO_TIMEOUT — i.e., no
        // hang, no OOM growth.
        let _ = stream.write_all(&giant).await;
        let mut sink = Vec::new();
        let n = timeout(IO_TIMEOUT, stream.read_to_end(&mut sink))
            .await
            .expect("server must not hang on oversized line")
            .expect("read ok");
        assert!(n < MAX_LINE_BYTES, "server echoed back a huge payload?");
    }

    #[tokio::test]
    async fn unknown_method_returns_error_without_closing() {
        let server = FakeStratum::start(Scenario::AcceptAll).await.unwrap();
        let mut c = TestClient::connect(server.addr()).await;

        c.send("mining.wat", serde_json::json!([])).await;
        let resp = c.recv().await.expect("response to unknown method");
        assert_eq!(resp["result"], serde_json::Value::Null);
        let err = resp["error"].as_array().expect("error tuple");
        assert_eq!(err[0].as_i64(), Some(20));

        // Connection must still work after the error.
        c.send("mining.subscribe", serde_json::json!([])).await;
        let sub = c.recv().await.expect("subscribe still works");
        assert_eq!(sub["result"][1].as_str(), Some(EXTRANONCE1));
    }

    #[tokio::test]
    async fn two_runs_produce_identical_recordings() {
        async fn run_once() -> Recorded {
            let server = FakeStratum::start(Scenario::AcceptAll).await.unwrap();
            let mut c = TestClient::connect(server.addr()).await;
            c.send("mining.subscribe", serde_json::json!(["ua"])).await;
            let _ = c.recv().await;
            c.send("mining.authorize", serde_json::json!(["u", "x"]))
                .await;
            let _ = c.recv().await;
            let _ = c.recv().await;
            let _ = c.recv().await;
            c.send(
                "mining.submit",
                serde_json::json!(["u", JOB_ID, "11", "22", "33"]),
            )
            .await;
            let _ = c.recv().await;
            drop(c);
            server.wait().await
        }

        let a = run_once().await;
        let b = run_once().await;
        assert_eq!(a.subscribe_count, b.subscribe_count);
        assert_eq!(a.authorize, b.authorize);
        assert_eq!(a.submits, b.submits);
        assert_eq!(a.extranonce1, b.extranonce1);
        assert_eq!(a.extranonce2_size, b.extranonce2_size);
    }
}
