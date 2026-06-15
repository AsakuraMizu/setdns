#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
IMAGE=${SETDNS_TEST_RUNNER_IMAGE:-setdns-test-runner:local}

cd "$ROOT"

docker build -f test-runner/docker/Dockerfile -t "$IMAGE" .

COMMON_ARGS=(
  --rm
  --privileged
  --cap-add NET_ADMIN
  --device /dev/net/tun
  -v "$ROOT:/repo"
  -w /repo
)

run_cell() {
  local name=$1
  local entrypoint=$2

  echo "==> Running ${name}"
  docker run "${COMMON_ARGS[@]}" --name "setdns-${name}" "$IMAGE" "$entrypoint"
}

run_cell bare-resolv-conf test-runner/docker/bare-entrypoint.sh
run_cell systemd-resolved test-runner/docker/resolved-entrypoint.sh
