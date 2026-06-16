#!/usr/bin/env bash
set -euo pipefail

source test-runner/platform/linux/docker/lib.sh

parent_dns=$(find_parent_dns)
warm_test_runner

snapshot_resolv_conf
expect_pass "bare global" global --parent-dns "$parent_dns"
assert_resolv_conf_unchanged
assert_no_setdns_residue

snapshot_resolv_conf
expect_pass "bare global through tun" global --tun --parent-dns "$parent_dns"
assert_resolv_conf_unchanged
assert_no_setdns_residue

snapshot_resolv_conf
expect_exit "bare split without systemd-resolved" 2 split --tun --parent-dns "$parent_dns"
assert_resolv_conf_unchanged
assert_no_setdns_residue
