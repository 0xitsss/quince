#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 0xitsss
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
#
# Release artifacts must be built from a fully described, version-aligned
# workspace. `--locked --no-deps` makes this an offline, deterministic check.
set -euo pipefail

metadata="$(cargo metadata --locked --no-deps --format-version 1)"
workspace_version="$(jq -r '.packages[] | select(.name == "quince") | .version' <<<"$metadata")"

if [[ -z "$workspace_version" || "$workspace_version" == "null" ]]; then
  printf 'release metadata gate: unable to resolve the quince package version\n' >&2
  exit 1
fi

failed=0
while IFS=$'\t' read -r name version edition license; do
  if [[ "$version" != "$workspace_version" ]]; then
    printf 'release metadata gate: %s version %s does not match workspace version %s\n' \
      "$name" "$version" "$workspace_version" >&2
    failed=1
  fi
  if [[ -z "$edition" || "$edition" == "null" ]]; then
    printf 'release metadata gate: %s is missing an edition\n' "$name" >&2
    failed=1
  fi
  if [[ -z "$license" || "$license" == "null" ]]; then
    printf 'release metadata gate: %s is missing SPDX license metadata\n' "$name" >&2
    failed=1
  fi
done < <(jq -r '.packages[] | [.name, .version, .edition, (.license // "")] | @tsv' <<<"$metadata")

if (( failed )); then
  exit 1
fi
