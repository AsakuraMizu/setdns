#!/usr/bin/env bash
set -euo pipefail

source test-runner/platform/linux/docker/lib.sh

parent_dns=$(find_parent_dns)
warm_test_runner
write_stub_resolv_conf_file

snapshot_resolv_conf
expect_exit "split with regular resolved stub file but without systemd-resolved" 2 split --tun --parent-dns "$parent_dns"
assert_resolv_conf_unchanged
assert_no_setdns_residue

snapshot_resolv_conf
expect_exit "global without device on regular resolved stub file without systemd-resolved" 2 global --parent-dns "$parent_dns"
assert_resolv_conf_unchanged
assert_no_setdns_residue
