#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 0xitsss
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
# Keep the public native-indicator catalogue in lockstep with compiled plugins.
set -euo pipefail

catalogue='book/src/native-indicators.md'
source_dir='indicators/src/custom'

if [[ ! -f "$catalogue" ]]; then
    echo "missing native indicator catalogue: $catalogue" >&2
    exit 1
fi

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT

find "$source_dir" -maxdepth 1 -type f -name '*.rs' -print0 \
    | xargs -0 rg --no-filename -o 'name\s*:\s*"(custom_[a-z_]+|signed_volume)"' \
    | sed -E 's/.*"([^"]+)"/\1/' \
    | sort -u > "$work_dir/registered"

rg -o '^\| `(custom_[a-z_]+|signed_volume)`' "$catalogue" \
    | sed -E 's/^\| `([^`]+)`.*/\1/' \
    | sort -u > "$work_dir/documented"

if ! diff -u "$work_dir/registered" "$work_dir/documented"; then
    echo "native indicator catalogue is out of sync with the compiled registry" >&2
    exit 1
fi

echo "native indicator catalogue: $(wc -l < "$work_dir/registered") registered indicators documented"
