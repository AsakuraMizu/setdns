#!/usr/bin/env bash
set -euo pipefail

source test-runner/platform/linux/docker/lib.sh

parent_dns=$(find_parent_dns)
warm_test_runner
write_managed_stub_resolv_conf

snapshot_resolv_conf
expect_exit "split with resolved stub but without systemd-resolved" 2 split --tun --parent-dns "$parent_dns"
assert_resolv_conf_unchanged
assert_no_setdns_residue

snapshot_resolv_conf
expect_exit "global without device on resolved-managed resolv.conf without systemd-resolved" 2 global --parent-dns "$parent_dns"
assert_resolv_conf_unchanged
assert_no_setdns_residue
