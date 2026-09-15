#!/usr/bin/env bash
# Fails when any given binary loads OpenSSL (libssl or libcrypto) dynamically.
# Usage: scripts/ci/check-no-dynamic-openssl.sh <binary>...

set -euo pipefail

[[ $# -gt 0 ]] || { echo "usage: check-no-dynamic-openssl.sh <binary>..." >&2; exit 2; }

status=0
for file in "$@"; do
  [[ -f "$file" ]] || { echo "error: $file does not exist" >&2; status=1; continue; }
  if [[ "$(uname)" == Darwin ]]; then
    deps="$(otool -L "$file")"
  else
    deps="$(readelf -d "$file" | grep NEEDED || true)"
  fi
  if grep -Ei 'libssl|libcrypto' <<<"$deps"; then
    echo "error: $file links OpenSSL dynamically" >&2
    status=1
  else
    echo "ok: $file has no dynamic OpenSSL"
  fi
done
exit "$status"
