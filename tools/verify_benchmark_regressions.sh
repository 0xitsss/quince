#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 0xitsss
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
#
# Criterion stores comparisons outside Git. Fail closed on a statistically
# significant mean regression above the configured threshold.
set -euo pipefail

readonly threshold="${QUINCE_BENCHMARK_MAX_REGRESSION:-0.10}"
marker=""

usage() {
  printf 'usage: %s [--since MARKER]\n' "$0" >&2
}

while (($# > 0)); do
  case "$1" in
    --since)
      shift
      marker="${1:-}"
      [[ -n "$marker" && -e "$marker" ]] || {
        printf 'benchmark marker does not exist: %s\n' "$marker" >&2
        exit 2
      }
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      usage
      exit 2
      ;;
  esac
  shift
done

# Cargo workspaces write Criterion output to the workspace target directory.
# Scanning crate-local target trees reads stale developer artefacts and can
# reject an unrelated change long after their baseline was superseded. The
# override exists only so the gate itself can be tested against fixtures.
readonly roots=("${QUINCE_BENCHMARK_ROOT:-target/criterion}")
failed=0
checked=0
bootstrapped=0

for root in "${roots[@]}"; do
  [[ -d "$root" ]] || continue
  while IFS= read -r estimate; do
    [[ -z "$marker" || "$estimate" -nt "$marker" ]] || continue
    lower_bound="$(jq -r '.mean.confidence_interval.lower_bound' "$estimate")"
    checked=$((checked + 1))
    # Reject only when the complete 95% confidence interval exceeds the
    # budget. Testing the upper bound rejected noisy samples whose estimated
    # mean and lower bound were still inside the budget.
    if awk -v value="$lower_bound" -v limit="$threshold" 'BEGIN { exit !(value > limit) }'; then
      benchmark="${estimate#"$root"/}"
      benchmark="${benchmark%/change/estimates.json}"
      printf 'benchmark regression exceeds %.1f%%: %s (lower bound %.2f%%)\n' \
        "$(awk -v limit="$threshold" 'BEGIN { print limit * 100 }')" \
        "$benchmark" \
        "$(awk -v value="$lower_bound" 'BEGIN { print value * 100 }')" >&2
      failed=1
    fi
  done < <(find "$root" -path '*/change/estimates.json' -type f -print)

  if [[ -n "$marker" ]]; then
    while IFS= read -r estimate; do
      [[ "$estimate" -nt "$marker" ]] || continue
      change="${estimate%/new/estimates.json}/change/estimates.json"
      if [[ ! -f "$change" || ! "$change" -nt "$marker" ]]; then
        printf 'benchmark baseline bootstrap: %s\n' "${estimate%/new/estimates.json}" >&2
        bootstrapped=$((bootstrapped + 1))
      fi
    done < <(find "$root" -path '*/new/estimates.json' -type f -print)
  fi
done

if [[ -n "$marker" && "$checked" -eq 0 && "$bootstrapped" -eq 0 ]]; then
  printf 'benchmark regression gate found no estimates from this run\n' >&2
  failed=1
fi

if [[ "$bootstrapped" -gt 0 && "${QUINCE_BENCHMARK_ALLOW_BOOTSTRAP:-0}" != "1" ]]; then
  printf 'benchmark baseline is missing for %d benchmark(s); bootstrap is allowed only on master\n' "$bootstrapped" >&2
  failed=1
fi

if (( failed )); then
  printf 'Benchmark regression gate failed. Investigate or explicitly version the private CI baseline.\n' >&2
  exit 1
fi
