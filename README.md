# Secure Community Bitcoin Miner

A minimal, Docker-first CPU solo Bitcoin miner. One hardened container runs a
pinned `pooler/cpuminer` SHA-256d engine plus a Rust supervisor and a local
dashboard. The dashboard reports real pool and Bitcoin-network data; it never
simulates earnings.

CPU solo mining is a lottery, not an income strategy. A CPU hashes in MH/s
while the Bitcoin network hashes in EH/s, so the chance of finding a block is
effectively zero. This release has **no GPU support**.

## Quick start

Clone [this repository](https://github.com/OvictorVieira/miner), then build the
image from the checked-out source:

```bash
printf 'WALLET=YOUR_CHECKSUM_VALID_BITCOIN_ADDRESS\n' > .env
docker compose build
docker compose up -d --no-build
```

Open <http://127.0.0.1:3500>. Compose publishes only host loopback. The
`miner-local:dev` image is built locally from digest-pinned base images and
`pull_policy: never` prevents a registry fallback. The quick start does not use
`latest`, any other mutable registry tag, or an unreviewed prebuilt miner image.

Check your worker at <https://web.public-pool.io/#/app/YOUR_WALLET>.

## Auditable security and privacy contract

The following are runtime guarantees for the supplied release Compose profile.
Each guarantee names its controlling threat-model section and its executable
verification. Run all repository checks with `cargo test --locked`.

| Guarantee | Control | Verification |
|---|---|---|
| The dashboard is local-only: Compose publishes `127.0.0.1:3500`, and a directly run binary refuses non-loopback binds. | [Dashboard exposure](SECURITY.md#dashboard-exposure) | [`published_port_is_loopback_only` and bind-policy tests](src/compose_policy.rs): `cargo test published_port_is_loopback_only --locked` and `cargo test bind_refuses --locked` |
| Dashboard pages load scripts and styles only from their own origin and send no browser request to a third party. | [Dashboard exposure](SECURITY.md#dashboard-exposure) | [`assets_reference_no_third_party_subresources`](src/main.rs): `cargo test assets_reference_no_third_party_subresources --locked` |
| The application sends no telemetry, analytics, crash report, donation share, or data to this project's maintainers. Its complete runtime egress is the three destinations below. | [Network and privacy boundaries](SECURITY.md#network-and-privacy-boundaries) | [`runtime_egress_is_explicit_and_bounded`](src/readme_policy.rs): `cargo test runtime_egress_is_explicit_and_bounded --locked` |
| The release container is non-root, read-only, capability-free, has no host mounts/devices/namespaces, and has bounded resources and logs. | [Host isolation](SECURITY.md#host-isolation) | [Compose policy tests](src/compose_policy.rs): `cargo test compose_policy --locked`; with Docker: `tests/compose_security.sh` |
| Only a checksum-valid public payout address and a bounded worker name form the mining identity; private keys, WIFs, and seed phrases are neither requested nor accepted. | [Credentials and payout identity](SECURITY.md#credentials-and-payout-identity) | [Address and identity tests](src/payout_identity.rs): `cargo test payout_identity --locked` and `cargo test bitcoin_address --locked` |
| Base images and cpuminer source are immutable inputs, and CI release images use exact SemVer tags rather than `latest`. | [Supply chain and image contents](SECURITY.md#supply-chain-and-image-contents) | [Dockerfile and CI policy tests](src/dockerfile_policy.rs): `cargo test dockerfile_policy --locked` and `cargo test ci_policy --locked` |

### Runtime outbound connections

There are exactly three network destinations in the default runtime:

| Destination | Transport | Data sent and purpose |
|---|---|---|
| `public-pool.io:21496` | **Plaintext** Stratum TCP | cpuminer subscribe/authorize/share messages. The public payout address and worker name are visible on the network; submitted work is also sent. The bundled cpuminer has no TLS support. |
| `https://public-pool.io:40557/api/client/<address>` | HTTPS | The validated public payout address, as a percent-encoded path segment, to retrieve worker and share statistics for the local dashboard. |
| `https://mempool.space/api` | HTTPS | Fixed requests for public price, block-height, and recent-block data. No payout address, worker name, password, or stable installation identifier is added. |

Nothing is sent to the maintainers. DNS resolution and ordinary protocol
metadata still reveal these destinations to the host's DNS resolver and network
operator. The HTTP client identifies only the application name/version in its
User-Agent. These boundaries are documented in the
[security threat model](SECURITY.md#network-and-privacy-boundaries) and pinned
in [`src/stats.rs`](src/stats.rs) and [`src/config.rs`](src/config.rs).

Stratum is intentionally configured as plaintext because the bundled cpuminer
does not support TLS. It exposes only public mining identity—not wallet secrets—but
an on-path observer can read or tamper with mining traffic. **Never enter or
mount a seed phrase, private key, or WIF.** This project will never request one.

### Dashboard reachability

The dashboard is reachable only from the host that runs the container—not from
the LAN, another host, or a public IP. Outside Docker, the binary also defaults
to `127.0.0.1` and rejects a non-loopback `BIND_ADDRESS`. Remote/LAN access is
out of scope in this release. Do not change the host side of the port mapping
to `0.0.0.0`, a LAN IP, or `::`.

`DASHBOARD_PASSWORD` is optional defense in depth for this local dashboard, not
a safe way to expose it remotely. It does not add TLS or multi-user isolation.
Sessions expire after at most 12 hours and are cleared on container restart.
See the [dashboard control](SECURITY.md#dashboard-exposure).

### Residual risk

Containers share the host kernel. A kernel, container-runtime, cpuminer, Rust
dependency, or Docker configuration vulnerability can still compromise the
container or host. Plaintext Stratum can be observed or modified in transit.
Resource limits reduce denial-of-service impact but cannot make hostile native
code safe. Do not run this beside sensitive workloads; for stronger isolation,
use the threat model's [disposable-VM recommendation](SECURITY.md#disposable-vm-recommendation).

## Configuration

| Variable | Default | Description |
|---|---|---|
| `WALLET` | — | **Required.** Checksum-valid public BTC payout address; never a private key or seed phrase |
| `POWER` | `50` | Percentage of CPU cores used by the miner (1–100) |
| `WORKER_NAME` | `miner` | Pool-visible worker name: 1–64 ASCII letters, digits, `_`, or `-` |
| `PORT` | `3500` | Container-internal dashboard port; Compose keeps the host publication fixed at `127.0.0.1:3500` |
| `DASHBOARD_PASSWORD` | — | Optional local-dashboard password; without it the loopback panel is read-only and unauthenticated |

To cap resource use beyond `POWER`, change Docker's `CPU_LIMIT` from its
default of `2` whole CPUs.

### Advanced typed configuration

Defaults are correct for almost everyone.

| Variable | Default | Description |
|---|---|---|
| `MODE` | `solo` | Only `solo` is implemented; `shared` is rejected explicitly |
| `NETWORK` | `mainnet` | Address network: `mainnet` or `testnet` |
| `POOL_URL` | `stratum+tcp://public-pool.io:21496` | Advanced endpoint override; must be `stratum+tcp://DNS_HOSTNAME:PORT` without credentials, IP literal, path, query, fragment, or whitespace |
| `POOL_USERNAME` | `<WALLET>.<WORKER_NAME>` | Advanced override for the public Stratum username |
| `SECRET_FILE` | — | Reserved for unimplemented shared-pool authentication; unused in solo mode |
| `TLS_POLICY` | `plaintext` | `required` is rejected because the bundled engine has no TLS support |

## What is in the image

The final image is based on the official `debian:bookworm-slim` image pinned to
`sha256:7c7b2c966bc9ee8cedfeef67e0e279108992c77681fa595db4a9d65c06ccc587`.
It contains:

- Debian slim's base files plus only the runtime packages `libcurl4`, `curl`,
  and `ca-certificates` and the unprivileged numeric user/group;
- `/usr/local/bin/minerd`, compiled during the build from
  [`pooler/cpuminer`](https://github.com/pooler/cpuminer) commit
  `8da0556cec32819d967734527a8e0f1d8efb0671`, with that commit recorded in
  `/usr/local/share/cpuminer-commit` and an image label;
- `/usr/local/bin/miner`, compiled from this repository's Rust source and
  embedded local assets using the locked `Cargo.lock`.

Nothing else from either build stage is copied: no compiler, Git checkout,
source tree, package cache, private key, seed phrase, wallet file, or build
credential is included. The complete construction is the
[`Dockerfile`](Dockerfile), controlled by [supply-chain policy](SECURITY.md#supply-chain-and-image-contents).

Build and inspect it locally:

```bash
docker compose build
docker history --no-trunc miner-local:dev
docker run --rm --entrypoint dpkg miner-local:dev -l
docker run --rm --entrypoint sh miner-local:dev -c \
  'cat /usr/local/share/cpuminer-commit; ls -l /usr/local/bin/miner /usr/local/bin/minerd'
```

`docker history` shows image layers, not a cryptographic inventory. For the
strongest review, inspect the Dockerfile and rebuild from a trusted checkout.

## Architecture

- **Hash engine:** pinned pooler/cpuminer `minerd`, built from source for
  amd64 and arm64 and restricted to SHA-256d.
- **Supervisor/dashboard:** a Rust binary (axum + tokio) that starts the engine
  without a shell, restarts it with bounded backoff, fetches display data, and
  serves `/health` plus the local panel.
- **Shutdown:** `SIGTERM` stops both the miner and HTTP server.

The release profile rejects `MINER_BIN` and `MINER_ARGS`; host-mounted engines
are unsupported. Contributors can explicitly opt into
`MINER_PROFILE=development` for local engine work, but that profile is not a
release configuration.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo run -- --self-test
```

## Release verification

From a fresh checkout with Docker running, execute the complete local release
gate:

```bash
tests/release_verification.sh
```

The gate builds a non-shipping verification stage from the pinned Docker
inputs and runs the fake-Stratum payout-identity flow and dashboard same-origin
asset policy inside it. It then builds `miner-local:dev`, starts an inert
service through Compose, inspects the effective loopback publication, numeric
non-root user, read-only root filesystem, dropped capabilities, and absence of
mounts, and runs the SHA-256d self-test from the final image with networking
disabled.

The final real-pool check is intentionally manual because it sends the public
payout identity and mining shares over plaintext Stratum:

1. Set a checksum-valid `WALLET` and a distinctive safe `WORKER_NAME` in
   `.env`, then run `docker compose build && docker compose up -d --no-build`.
2. Leave the miner running for at least 30 minutes. Record the start/end time,
   image ID (`docker image inspect miner-local:dev --format '{{.Id}}'`), and
   accepted-share lines from `docker compose logs miner`.
3. Open `https://web.public-pool.io/#/app/YOUR_WALLET`, confirm the distinctive
   worker is visible, and record the worker name plus accepted-share count.
4. Stop the service with `docker compose down`. Treat a missing/mismatched
   worker or no accepted shares as a failed release check to investigate, not
   as permission to weaken the payout-identity or network controls.

See [SECURITY.md](SECURITY.md) for the complete threat model and private
vulnerability-reporting process.

## License

MIT
