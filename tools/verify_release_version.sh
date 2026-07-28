#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 0xitsss
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
#
# A release tag is an externally visible promise. Never publish binaries whose
# Cargo package version differs from the version encoded in that tag.
set -euo pipefail

if (( $# != 1 )); then
  printf 'usage: %s <release-version>\n' "$0" >&2
  exit 2
fi

release_version="${1#v}"
semver_pattern='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$'
if [[ ! "$release_version" =~ $semver_pattern ]]; then
  printf 'release version gate: %q is not a valid SemVer version\n' "$1" >&2
  exit 1
fi

metadata="$(cargo metadata --locked --no-deps --format-version 1)"
package_version="$(jq -r '.packages[] | select(.name == "quince") | .version' <<<"$metadata")"
if [[ -z "$package_version" || "$package_version" == "null" ]]; then
  printf 'release version gate: unable to resolve the quince package version\n' >&2
  exit 1
fi

if [[ "$package_version" != "$release_version" ]]; then
  printf 'release version gate: requested version %s does not match quince package version %s\n' \
    "$release_version" "$package_version" >&2
  exit 1
fi
