#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 0xitsss
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$repo_root/tools/verify_release_version.sh"
current_version="$(cargo metadata --locked --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "quince") | .version')"

bash "$script" "$current_version"
bash "$script" "v$current_version"

if bash "$script" "0.0.0" >/dev/null 2>&1; then
  printf 'release version gate test: mismatched version unexpectedly passed\n' >&2
  exit 1
fi

if bash "$script" "not-a-version" >/dev/null 2>&1; then
  printf 'release version gate test: malformed version unexpectedly passed\n' >&2
  exit 1
fi

if bash "$script" >/dev/null 2>&1; then
  printf 'release version gate test: missing version unexpectedly passed\n' >&2
  exit 1
fi
