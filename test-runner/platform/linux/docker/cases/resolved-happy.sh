#!/usr/bin/env bash
set -euo pipefail

source test-runner/platform/linux/docker/lib.sh

parent_dns=$(find_parent_dns)
warm_test_runner
start_resolved yes "$parent_dns"
write_stub_resolv_conf_file

snapshot_resolv_conf
expect_pass "resolved global through tun" global --tun --parent-dns "$parent_dns"
assert_resolv_conf_unchanged
assert_no_setdns_residue

snapshot_resolv_conf
expect_pass "resolved split through tun" split --tun --parent-dns "$parent_dns"
assert_resolv_conf_unchanged
assert_no_setdns_residue
