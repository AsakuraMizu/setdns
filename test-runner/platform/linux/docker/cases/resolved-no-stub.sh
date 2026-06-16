#!/usr/bin/env bash
set -euo pipefail

source test-runner/platform/linux/docker/lib.sh

parent_dns=$(find_parent_dns)
warm_test_runner
start_resolved no "$parent_dns"
write_parent_resolv_conf_file "$parent_dns"

snapshot_resolv_conf
expect_exit "split with systemd-resolved but without stub resolver" 1 split --tun --parent-dns "$parent_dns"
assert_resolv_conf_unchanged
assert_no_setdns_residue
