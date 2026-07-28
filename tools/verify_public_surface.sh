#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 0xitsss
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
#
# Keep ad-hoc reports, post drafts, plans, and benchmark output out of the
# public repository. Canonical product documentation is intentionally small.
set -euo pipefail

readonly allowed_docs=(
  'docs/QFL.md'
  'docs/QUINCE.md'
)

is_allowed_doc() {
  local path="$1"
  local allowed
  for allowed in "${allowed_docs[@]}"; do
    [[ "$path" == "$allowed" ]] && return 0
  done
  return 1
}

while IFS= read -r path; do
  if ! is_allowed_doc "$path"; then
    printf 'unexpected public documentation: %s\n' "$path" >&2
    printf 'Use an allowed canonical document or keep the material outside Git.\n' >&2
    exit 1
  fi
done < <(git ls-files 'docs/*.md' 'docs/*.MD')
