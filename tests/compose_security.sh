#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
compose_project="miner-security-${GITHUB_RUN_ID:-$$}"
compose_files=(-f "$repo_dir/docker-compose.yml" -f "$repo_dir/tests/docker-compose.security-test.yml")
export WALLET="1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"

cleanup() {
  docker compose --project-name "$compose_project" "${compose_files[@]}" \
    down --volumes --remove-orphans >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker compose --project-name "$compose_project" "${compose_files[@]}" config --quiet
docker compose --project-name "$compose_project" "${compose_files[@]}" build
docker compose --project-name "$compose_project" "${compose_files[@]}" up -d --no-build

container_id="$(docker compose --project-name "$compose_project" "${compose_files[@]}" ps -q miner)"
if [[ -z "$container_id" ]]; then
  echo "miner container was not created" >&2
  exit 1
fi

inspection_file="$(mktemp)"
trap 'rm -f "$inspection_file"; cleanup' EXIT
docker inspect "$container_id" >"$inspection_file"

seccomp_mode="$(docker exec "$container_id" sh -c "awk '/^Seccomp:/{print \$2}' /proc/self/status")"
if [[ "$seccomp_mode" != "2" ]]; then
  echo "container must run with seccomp filtering enabled (mode 2), got ${seccomp_mode:-missing}" >&2
  exit 1
fi

python3 - "$inspection_file" <<'PY'
import json
import pathlib
import sys

container = json.loads(pathlib.Path(sys.argv[1]).read_text())[0]
config = container["Config"]
host = container["HostConfig"]
state = container["State"]

def require(condition, message):
    if not condition:
        raise AssertionError(message)

user = config.get("User", "")
parts = user.split(":")
require(len(parts) == 2 and all(part.isdigit() and int(part) > 0 for part in parts),
        f"container user must be a non-root numeric UID:GID, got {user!r}")
require(state.get("Running") is True, "security test must inspect a running container")
require(host.get("ReadonlyRootfs") is True, "root filesystem must be read-only")
require("ALL" in (host.get("CapDrop") or []), "all Linux capabilities must be dropped")

security_opt = [value.lower() for value in (host.get("SecurityOpt") or [])]
require("no-new-privileges:true" in security_opt, "no-new-privileges must be active")
require(not any("seccomp=unconfined" in value or "seccomp:unconfined" in value
                for value in security_opt), "seccomp must not be unconfined")

require(host.get("Privileged") is False, "privileged mode must be disabled")
require(host.get("NetworkMode") != "host", "host network namespace is forbidden")
require(host.get("PidMode") != "host", "host PID namespace is forbidden")
require(host.get("IpcMode") != "host", "host IPC namespace is forbidden")
require(host.get("UsernsMode") != "host", "host user namespace is forbidden")
require(not host.get("Devices"), "host devices are forbidden")
require(not host.get("DeviceRequests"), "device requests are forbidden")
require(not host.get("Binds"), "host bind mounts and control sockets are forbidden")
require(not container.get("Mounts"), "named volumes and host mounts are forbidden")

require(host.get("NanoCpus", 0) > 0, "CPU limit must be active")
require(host.get("Memory", 0) > 0, "memory limit must be active")
require(host.get("PidsLimit", 0) > 0, "PID limit must be active")
require(host.get("Tmpfs"), "bounded tmpfs must be active")
for destination, options in host["Tmpfs"].items():
    normalized = options.lower()
    require("size=" in normalized, f"tmpfs {destination} must have a size limit")
    for flag in ("noexec", "nosuid", "nodev"):
        require(flag in normalized, f"tmpfs {destination} must set {flag}")

ulimits = {item["Name"]: item for item in (host.get("Ulimits") or [])}
require(ulimits.get("core", {}).get("Soft") == 0, "core soft limit must be zero")
require(ulimits.get("core", {}).get("Hard") == 0, "core hard limit must be zero")
require("nofile" in ulimits, "file-descriptor limit must be active")

log_config = host.get("LogConfig") or {}
require(log_config.get("Type") == "json-file", "json-file log driver must be active")
require(log_config.get("Config", {}).get("max-size"), "log max-size must be active")
require(log_config.get("Config", {}).get("max-file"), "log max-file must be active")

bindings = host.get("PortBindings") or {}
require(bindings, "dashboard port must be published")
for entries in bindings.values():
    require(entries, "published port must have a host binding")
    for entry in entries:
        require(entry.get("HostIp") == "127.0.0.1",
                f"published port must use 127.0.0.1, got {entry!r}")

require(config.get("Image") == "miner-local:dev",
        f"runtime image must be local-only, got {config.get('Image')!r}")
print("runtime Compose security inspection passed")
PY
