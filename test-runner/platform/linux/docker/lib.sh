#!/usr/bin/env bash

export RUST_LOG=${RUST_LOG:-info}

RESOLV_CONF=/etc/resolv.conf
SETDNS_BACKUP=/etc/resolv.conf.setdns.bak
SETDNS_TEMP=/etc/resolv.conf.setdns.tmp
RUNNER=(cargo run --manifest-path test-runner/Cargo.toml --)
OFFLINE_RUNNER=(cargo run --offline --manifest-path test-runner/Cargo.toml --)
RESOLV_CONF_SNAPSHOT=
RESOLVED_PID=

find_parent_dns() {
  local parent_dns
  parent_dns=$(awk '$1 == "nameserver" && $2 != "127.0.0.53" && $2 != "127.0.0.54" && $2 != "127.0.0.1" { print $2; exit }' "$RESOLV_CONF")
  if [[ -z "$parent_dns" ]]; then
    echo "No non-stub parent DNS server found in $RESOLV_CONF" >&2
    exit 2
  fi
  printf '%s\n' "$parent_dns"
}

warm_test_runner() {
  cargo build --manifest-path test-runner/Cargo.toml
  RUNNER=("${OFFLINE_RUNNER[@]}")
}

expect_pass() {
  local name=$1
  shift

  echo "==> Expect pass: $name"
  set +e
  "${RUNNER[@]}" "$@"
  local status=$?
  set -e

  if [[ "$status" -ne 0 ]]; then
    echo "Expected '$name' to pass, got exit $status" >&2
    return 1
  fi
}

expect_exit() {
  local name=$1
  local expected=$2
  shift 2

  echo "==> Expect exit $expected: $name"
  set +e
  "${RUNNER[@]}" "$@"
  local status=$?
  set -e

  if [[ "$status" -ne "$expected" ]]; then
    echo "Expected '$name' to exit $expected, got $status" >&2
    return 1
  fi
}

snapshot_resolv_conf() {
  RESOLV_CONF_SNAPSHOT=$(mktemp)
  describe_resolv_conf >"$RESOLV_CONF_SNAPSHOT"
}

assert_resolv_conf_unchanged() {
  local current
  current=$(mktemp)
  describe_resolv_conf >"$current"

  if ! diff -u "$RESOLV_CONF_SNAPSHOT" "$current"; then
    echo "$RESOLV_CONF changed unexpectedly" >&2
    return 1
  fi
}

assert_no_setdns_residue() {
  if [[ -e "$SETDNS_BACKUP" ]]; then
    echo "setdns backup remains at $SETDNS_BACKUP" >&2
    return 1
  fi
  if [[ -e "$SETDNS_TEMP" ]]; then
    echo "setdns temp file remains at $SETDNS_TEMP" >&2
    return 1
  fi

  local first_line=
  if [[ -r "$RESOLV_CONF" ]]; then
    IFS= read -r first_line <"$RESOLV_CONF" || true
  fi
  if [[ "$first_line" == "# setdns owner: "* ]]; then
    echo "setdns owner header remains in $RESOLV_CONF" >&2
    return 1
  fi
}

describe_resolv_conf() {
  if [[ -L "$RESOLV_CONF" ]]; then
    echo "type=symlink"
    printf 'target=%s\n' "$(readlink "$RESOLV_CONF")"
    echo "content-begin"
    if [[ -e "$RESOLV_CONF" ]]; then
      cat "$RESOLV_CONF"
    else
      echo "missing-target"
    fi
    echo "content-end"
    return
  fi

  if [[ -e "$RESOLV_CONF" ]]; then
    echo "type=file"
    echo "content-begin"
    cat "$RESOLV_CONF"
    echo "content-end"
    return
  fi

  echo "type=missing"
}

start_dbus() {
  mkdir -p /run/dbus
  dbus-daemon --system --fork
}

start_resolved() {
  local stub_listener=$1
  local parent_dns=$2

  mkdir -p /run/dbus /etc/systemd/resolved.conf.d
  cat >/etc/systemd/resolved.conf.d/setdns-test-runner.conf <<EOF
[Resolve]
DNS=$parent_dns
DNSStubListener=$stub_listener
EOF

  start_dbus

  local resolved_bin=/lib/systemd/systemd-resolved
  if [[ ! -x "$resolved_bin" ]]; then
    resolved_bin=/usr/lib/systemd/systemd-resolved
  fi

  "$resolved_bin" &
  RESOLVED_PID=$!
  trap cleanup_resolved EXIT
  wait_for_resolved
}

wait_for_resolved() {
  local resolved_ready=0
  for _ in {1..50}; do
    if dbus-send --system --dest=org.freedesktop.DBus --print-reply /org/freedesktop/DBus org.freedesktop.DBus.NameHasOwner string:org.freedesktop.resolve1 | grep -q 'boolean true'; then
      resolved_ready=1
      break
    fi
    sleep 0.1
  done

  if [[ "$resolved_ready" -ne 1 ]]; then
    echo "systemd-resolved did not acquire org.freedesktop.resolve1 on the system bus" >&2
    exit 2
  fi
}

cleanup_resolved() {
  if [[ -n "$RESOLVED_PID" ]]; then
    kill "$RESOLVED_PID" 2>/dev/null || true
  fi
}

write_resolv_conf_file() {
  local content=$1
  if [[ -L "$RESOLV_CONF" ]]; then
    rm -f "$RESOLV_CONF"
  fi
  printf '%s' "$content" >"$RESOLV_CONF"
}

write_stub_resolv_conf_file() {
  write_resolv_conf_file $'nameserver 127.0.0.53\noptions edns0 trust-ad\n'
}

write_parent_resolv_conf_file() {
  local parent_dns=$1
  write_resolv_conf_file "nameserver $parent_dns"$'\n'
}

write_managed_stub_resolv_conf() {
  mkdir -p /run/systemd/resolve
  printf 'nameserver 127.0.0.53\noptions edns0 trust-ad\n' >/run/systemd/resolve/stub-resolv.conf

  if mountpoint -q "$RESOLV_CONF"; then
    umount "$RESOLV_CONF"
  fi
  rm -f "$RESOLV_CONF"
  ln -s /run/systemd/resolve/stub-resolv.conf "$RESOLV_CONF"
}
