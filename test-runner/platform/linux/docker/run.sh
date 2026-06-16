#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)
IMAGE=${SETDNS_TEST_RUNNER_IMAGE:-setdns-test-runner:linux-local}

cd "$ROOT"

docker build -f test-runner/platform/linux/docker/Dockerfile -t "$IMAGE" .

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
  local case_script=$2

  echo "==> Running ${name}"
  docker run "${COMMON_ARGS[@]}" --name "setdns-${name}" "$IMAGE" bash "$case_script"
}

run_cell bare-resolv-conf test-runner/platform/linux/docker/cases/bare.sh
run_cell systemd-resolved-happy test-runner/platform/linux/docker/cases/resolved-happy.sh
run_cell dbus-without-resolved test-runner/platform/linux/docker/cases/dbus-without-resolved.sh
run_cell stub-without-resolved test-runner/platform/linux/docker/cases/stub-without-resolved.sh
run_cell stub-file-without-resolved test-runner/platform/linux/docker/cases/stub-file-without-resolved.sh
run_cell resolved-no-stub test-runner/platform/linux/docker/cases/resolved-no-stub.sh
run_cell managed-resolv-conf-global-no-device test-runner/platform/linux/docker/cases/managed-resolv-conf-global-no-device.sh
