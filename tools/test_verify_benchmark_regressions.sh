#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 0xitsss
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
set -euo pipefail

workspace="$(pwd)"
fixture_root="$(mktemp -d)"
trap 'rm -rf "$fixture_root"' EXIT
estimate_dir="$fixture_root/criterion/fixture/change"
mkdir -p "$estimate_dir"

# A noisy interval crossing the budget must pass: the regression is not
# statistically established above the configured threshold.
printf '%s\n' \
  '{"mean":{"confidence_interval":{"lower_bound":0.05,"upper_bound":0.20}}}' \
  > "$estimate_dir/estimates.json"
QUINCE_BENCHMARK_ROOT="$fixture_root/criterion" \
  bash "$workspace/tools/verify_benchmark_regressions.sh"

# The complete interval above the budget must fail.
printf '%s\n' \
  '{"mean":{"confidence_interval":{"lower_bound":0.11,"upper_bound":0.20}}}' \
  > "$estimate_dir/estimates.json"
if QUINCE_BENCHMARK_ROOT="$fixture_root/criterion" \
  bash "$workspace/tools/verify_benchmark_regressions.sh" >/dev/null 2>&1; then
  printf 'benchmark regression fixture unexpectedly passed\n' >&2
  exit 1
fi

printf 'benchmark regression confidence-interval contract: ok\n'
