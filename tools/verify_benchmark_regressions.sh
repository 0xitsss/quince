#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 0xitsss
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
#
# Criterion stores comparisons outside Git. Fail closed on a statistically
# significant mean regression above the configured threshold.
set -euo pipefail

readonly threshold="${QUINCE_BENCHMARK_MAX_REGRESSION:-0.10}"
readonly roots=(qfl/target/criterion indicators/target/criterion engine/target/criterion)
failed=0

for root in "${roots[@]}"; do
  [[ -d "$root" ]] || continue
  while IFS= read -r estimate; do
    upper_bound="$(jq -r '.mean.confidence_interval.upper_bound' "$estimate")"
    if awk -v value="$upper_bound" -v limit="$threshold" 'BEGIN { exit !(value > limit) }'; then
      benchmark="${estimate#"$root"/}"
      benchmark="${benchmark%/change/estimates.json}"
      printf 'benchmark regression exceeds %.1f%%: %s (upper bound %.2f%%)\n' \
        "$(awk -v limit="$threshold" 'BEGIN { print limit * 100 }')" \
        "$benchmark" \
        "$(awk -v value="$upper_bound" 'BEGIN { print value * 100 }')" >&2
      failed=1
    fi
  done < <(find "$root" -path '*/change/estimates.json' -type f -print)
done

if (( failed )); then
  printf 'Benchmark regression gate failed. Investigate or explicitly version the private CI baseline.\n' >&2
  exit 1
fi
