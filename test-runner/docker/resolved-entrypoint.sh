#!/usr/bin/env bash
set -euo pipefail

export RUST_LOG=${RUST_LOG:-info}

parent_dns=$(awk '$1 == "nameserver" && $2 != "127.0.0.53" && $2 != "127.0.0.54" && $2 != "127.0.0.1" { print $2; exit }' /etc/resolv.conf)
if [[ -z "$parent_dns" ]]; then
  echo "No non-stub parent DNS server found in /etc/resolv.conf" >&2
  exit 2
fi

mkdir -p /run/dbus /etc/systemd/resolved.conf.d
cat >/etc/systemd/resolved.conf.d/setdns-test-runner.conf <<EOF
[Resolve]
DNS=$parent_dns
DNSStubListener=yes
EOF

dbus-daemon --system --fork

resolved_bin=/lib/systemd/systemd-resolved
if [[ ! -x "$resolved_bin" ]]; then
  resolved_bin=/usr/lib/systemd/systemd-resolved
fi
"$resolved_bin" &
resolved_pid=$!
trap 'kill "$resolved_pid" 2>/dev/null || true' EXIT

resolved_ready=0
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


printf 'nameserver 127.0.0.53\noptions edns0 trust-ad\n' >/etc/resolv.conf

cargo run --manifest-path test-runner/Cargo.toml -- global --tun --parent-dns "$parent_dns"
cargo run --manifest-path test-runner/Cargo.toml -- split --tun --parent-dns "$parent_dns"
