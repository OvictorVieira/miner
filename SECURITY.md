# Security Threat Model for Secure Community Bitcoin Miner

## Protected Assets
- User's Bitcoin wallet private keys (not stored in the container)
- Mining rewards (BTC payouts)
- Mining credentials (WALLET, POOL_URL)
- Dashboard access (if password protected)

## In-Scope Threats
- Container breakout attempts (privilege escalation, host compromise)
- Unauthorized access to mining credentials or rewards
- Network egress to untrusted endpoints
- Exploitation of dashboard or mining engine vulnerabilities
- Supply chain attacks via container images or dependencies

## Residual Risks (Container/Kernel/Runtime/GPU)
- Kernel vulnerabilities on the host (container escape)
- Host device exposure (e.g., GPUs, USB, PCI devices)
- Unpatched container runtime bugs
- GPU driver vulnerabilities (if using NVIDIA/AMD backends)
- Shared kernel namespaces (PID, network, IPC)

## Disposable-VM Recommendation
For maximum isolation, run the miner in a disposable VM or trusted, minimal host. Avoid running on shared or sensitive infrastructure.

## Prohibited Host Mounts, Sockets, Namespaces, Devices, Capabilities
- Do not mount host directories or devices into the container (no `volumes:` for host paths)
- Do not expose Docker socket or privileged host devices
- Do not run with `--privileged` or add extra Linux capabilities
- Do not share host network, PID, or IPC namespaces
- Avoid GPU passthrough unless required and trusted

## Vulnerability Reporting
- Please report vulnerabilities privately to the maintainers via email (see repository contact)
- Supported release versions: latest stable release and previous minor

## Documentation Links
- [README.md](./README.md)
- [docker-compose.yml](./docker-compose.yml)

---
