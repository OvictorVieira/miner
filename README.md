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
| `MINER_BIN` | `/usr/local/bin/minerd` | Miner binary |
| `MINER_ARGS` | — | Custom miner arguments (see below) |

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

## Pluggable engine (GPU, other miners)

The mining engine is swappable via env: mount your own binary into the
container and point `MINER_BIN` + `MINER_ARGS` at it. Inside the arguments,
`{POOL}`, `{USER}` and `{THREADS}` are substituted from the configuration:

```yaml
services:
  miner:
    image: ghcr.io/fullsystem/miner:latest
    volumes:
      - ./my-gpu-miner:/opt/gpu-miner:ro
    environment:
      WALLET: bc1q...
      MINER_BIN: /opt/gpu-miner
      MINER_ARGS: "--url {POOL} --user {USER} --pass x --gpu 0"
    # NVIDIA GPU: requires nvidia-container-toolkit on the host
    # deploy:
    #   resources:
    #     reservations:
    #       devices:
    #         - driver: nvidia
    #           count: all
    #           capabilities: [gpu]
```

The supervisor (backoff restarts, clean shutdown, `/health`) works the same
for any engine.

> **An honest note on GPU + Bitcoin**: GPUs lost the SHA-256d race to ASICs
> around 2013. A GPU improves your odds ~1000x over a CPU, but the lottery is
> still a lottery. This feature exists for flexibility (other algorithms,
> pools, miners) — not for economic viability on BTC.

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
