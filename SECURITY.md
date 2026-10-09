# Security Threat Model

This document defines the security boundary of the supplied release Dockerfile
and Compose profile. Development overrides are outside that boundary.

## Protected assets

- the host, its files, devices, container runtime, and other workloads;
- Bitcoin rewards and the public payout identity used to route them;
- optional pool and dashboard credentials;
- the integrity and privacy of the loopback-only dashboard.

Wallet private keys and seed phrases are deliberately not application assets:
the miner never needs, requests, stores, or accepts them.

## In-scope threats

- a compromised miner or dashboard process attempting privilege escalation,
  host access, persistence, or resource exhaustion;
- payout redirection through malformed configuration or an unintended fallback;
- accidental dashboard publication to a LAN or the internet;
- unwanted telemetry, third-party browser assets, or undocumented runtime
  network destinations;
- mutable container bases, engine source, dependencies, or CI actions changing
  a reviewed build.

## Host isolation

The release service runs as numeric UID/GID `10001:10001`, with a read-only
root filesystem, no Linux capabilities, `no-new-privileges`, Docker's default
seccomp policy, bounded CPU/memory/PIDs/file descriptors/logs, disabled core
dumps, and one bounded `/tmp` tmpfs. It has no host bind/named volume, socket,
device, privileged mode, or host PID/IPC/network/user namespace. Keep these
controls intact; `src/compose_policy.rs` and `tests/compose_security.sh` verify
the checked-in and effective policies.

### Prohibited release configuration

- no host directories, Docker/container-runtime sockets, or named volumes;
- no host devices, GPU passthrough, USB/PCI access, or device rules;
- no `--privileged`, added capability, `seccomp=unconfined`, or writable root;
- no host PID, IPC, network, or user namespace;
- no arbitrary host-mounted mining binary or free-form release arguments.

## Dashboard exposure

Compose fixes the host publication to `127.0.0.1:3500`. The process may bind
`0.0.0.0` only inside that container; outside a container it defaults to host
loopback and refuses every non-loopback bind. Remote/LAN access is unsupported.

The optional `DASHBOARD_PASSWORD` is local defense in depth, not permission to
publish remotely: there is no TLS or multi-user isolation. Login bodies are
bounded, failures have a uniform delay, secret comparison is constant-time,
and sessions expire within 12 hours. Every response has a restrictive CSP,
no-referrer/no-sniff/no-store headers, and pages load no third-party assets.

## Network and privacy boundaries

The application sends no telemetry, analytics, crash reports, donation shares,
or other data to project maintainers. Default runtime egress is limited to:

1. plaintext Stratum TCP at `public-pool.io:21496`, carrying the public payout
   address, worker name, protocol messages, and submitted work;
2. HTTPS at `public-pool.io:40557`, carrying the public payout address in the
   requested stats path;
3. HTTPS at `mempool.space`, requesting fixed public price and block data with
   no payout or worker identity.

The dashboard browser itself makes only same-origin requests. DNS and transport
metadata remain visible to the configured resolver and network operator. The
HTTP client sends the application name/version as its User-Agent. An advanced
`POOL_URL` override changes the Stratum destination only after strict parsing;
it does not change either HTTPS stats endpoint.

The bundled cpuminer does **not** support Stratum TLS. An on-path party can read
or alter its public identity, jobs, and shares. TLS policy `required` therefore
fails closed rather than pretending transport confidentiality exists.

## Credentials and payout identity

`WALLET` accepts only a checksum-valid public Bitcoin address for the selected
network. `WORKER_NAME` has a small ASCII allowlist and length bound. Together
they form the default Stratum username and are verified end to end by an
offline fake pool. There is no donation identity or fallback payout address.

Never provide, mount, or bake a seed phrase, wallet private key, WIF, wallet
file, exchange password, or unrelated secret. Anyone requesting one for this
miner is not following this project's design.

## Supply chain and image contents

Every Docker `FROM` uses a reviewed SHA-256 digest. cpuminer is checked out at
full commit `8da0556cec32819d967734527a8e0f1d8efb0671`, verified before compilation,
and recorded in the image. Rust uses `Cargo.lock`; CI actions use full commit
SHAs; published images receive only full SemVer tags, never `latest`.

The final stage starts from the official Debian bookworm-slim digest, installs
only `libcurl4`, `curl`, and `ca-certificates` as runtime packages, creates the
unprivileged user, and copies exactly the compiled `minerd`, its source-commit
record, and the compiled Rust `miner`. Build tools, source trees, and credentials
remain in discarded build stages. See the [image inventory and inspection
commands](README.md#what-is-in-the-image).

## Residual risks

- Containers share the host kernel; kernel and container-runtime flaws can
  bypass container controls.
- Native cpuminer and Rust dependencies can contain exploitable defects.
- Plaintext Stratum permits observation and tampering by an on-path party.
- Resource limits bound common failure modes but cannot make hostile code safe.
- The host controls Docker and the image: a hostile administrator can replace
  configuration, binaries, traffic, or displayed data.
- GPU support is absent. Adding device passthrough later would introduce a
  large driver/kernel attack surface and requires a new threat review.

## Disposable-VM recommendation

For stronger isolation, run the miner inside a disposable VM on a trusted,
minimal host with no sensitive data or workloads. Destroy the VM after use.
This reduces impact from a container escape but does not encrypt Stratum or
protect against a compromised hypervisor, host, pool, DNS resolver, or network.

## Vulnerability reporting

Report vulnerabilities privately through GitHub's **Security advisories →
Report a vulnerability** flow for this repository. Do not open a public issue
before a fix is available.

Supported versions are the latest stable release and its immediately preceding
minor release. Development branches and custom images are unsupported.

## Related documentation

- [README and auditable contract](README.md#auditable-security-and-privacy-contract)
- [Release Compose profile](docker-compose.yml)
- [Container build](Dockerfile)
