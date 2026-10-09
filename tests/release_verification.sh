#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
run_id="${GITHUB_RUN_ID:-$$}"
verification_image="miner-offline-verification:${run_id}"

cleanup() {
  docker image rm --force "$verification_image" >/dev/null 2>&1 || true
}
trap cleanup EXIT

# Build, but never ship, a stage that runs the fake-Stratum payout-identity and
# no-third-party browser-origin tests in the same locked builder tree as the
# release binary.
docker build \
  --target offline-verification \
  --tag "$verification_image" \
  "$repo_dir"

# Build the final image through Compose, start an inert container (so this check
# never contacts the real pool), and inspect Docker's effective hardening.
"$repo_dir/tests/compose_security.sh"

# Exercise the self-test from the actual stripped runtime image as its baked-in
# non-root user, with no network and the same core privilege restrictions.
docker run --rm \
  --pull never \
  --network none \
  --read-only \
  --cap-drop ALL \
  --security-opt no-new-privileges:true \
  --entrypoint /usr/local/bin/miner \
  miner-local:dev \
  --self-test

echo "local release end-to-end verification passed"
