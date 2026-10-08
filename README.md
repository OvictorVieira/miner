# miner

Docker-first solo Bitcoin miner ("lottery mining"): a single container, 100%
environment-variable configuration, and a dashboard that shows real data only.
Built to run on any homelab, VPS, or PaaS (Dokploy, etc.) alongside your other
projects.

## Usage

```bash
docker run -d \
  -e WALLET=bc1qyourwallet... \
  -e POWER=50 \
  -p 3500:3500 \
  ghcr.io/fullsystem/miner:latest
```

Or with compose: copy `.env.example` to `.env`, set `WALLET`, and run
`docker compose up -d`.

Check your worker at `https://web.public-pool.io/#/app/YOUR_WALLET`.

## Configuration (env)

| Variable | Default | Description |
|---|---|---|
| `WALLET` | — | **Required.** Your BTC payout address (receives the reward if you ever find a block) |
| `POWER` | `50` | % of CPU cores used by the miner (1-100) |
| `WORKER_NAME` | `miner` | Worker name shown at the pool (useful with multiple instances) |
| `PORT` | `3500` | Dashboard port |
| `DASHBOARD_PASSWORD` | — | Dashboard password; without it the panel is public read-only |

### Advanced (typed, validated; defaults are correct for almost everyone)

| Variable | Default | Description |
|---|---|---|
| `MODE` | `solo` | Mining topology. Only `solo` is implemented; `shared` is rejected with an explicit error |
| `NETWORK` | `mainnet` | Bitcoin network the payout address belongs to (`mainnet` or `testnet`) |
| `POOL_URL` | `stratum+tcp://public-pool.io:21496` | Solo pool endpoint (stratum). Overriding this is an advanced, mostly-unvalidated knob |
| `POOL_USERNAME` | `<WALLET>.<WORKER_NAME>` | Override the Stratum username sent to the pool instead of the computed default |
| `SECRET_FILE` | — | Path to a file holding a pool secret; reserved for shared-pool authentication, unused by `MODE=solo` |
| `TLS_POLICY` | `plaintext` | Stratum transport policy. `required` is rejected: the bundled cpuminer engine has no TLS support |

To cap resource usage beyond `POWER`, use Docker's own CPU limit
(`cpus: "2.0"` in compose).

## Engine

The release image runs the reviewed, image-baked
[pooler/cpuminer](https://github.com/pooler/cpuminer) exclusively. Setting
`MINER_BIN` or `MINER_ARGS` is rejected: host-mounted engines are a
development-only feature and have no supported release path. Contributors who
need to iterate against another engine opt in explicitly with
`MINER_PROFILE=development` — this is not for production.

## Architecture

- **Hash engine**: [pooler/cpuminer](https://github.com/pooler/cpuminer)
  (`minerd`), compiled from source during the image build — no prebuilt
  binaries in the repo, native amd64 and arm64 support.
- **Supervisor/dashboard**: a Rust binary (axum + tokio) that manages the
  miner process (exponential-backoff restarts), exposes `/health`, and serves
  the panel.
- **Clean shutdown**: `SIGTERM` gracefully stops both the miner and the server.

## Honest disclaimer

Solo-mining Bitcoin on a CPU is a true lottery: the odds of finding a block
are effectively zero (the network operates in EH/s; a CPU, in MH/s). Run it
for fun, learning, and to support decentralization — not for income.

## Development

```bash
cargo test
WALLET=bc1q... MINER_BIN=/path/to/minerd cargo run
```

## Support

If this project made you smile (or you actually hit a block — imagine),
donations are welcome:

```
bc1pwy2ulg769ffvhwchk4yzkcq5yq699qwrrkg3a4lq942rj47sutcq2xjny5
```

## License

MIT
