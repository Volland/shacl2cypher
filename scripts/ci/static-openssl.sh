#!/usr/bin/env bash
# Builds a static-only OpenSSL into <prefix>.
#
# lbug (LadybugDB) always links `ssl` and `crypto`. With OPENSSL_DIR=<prefix> its
# build script searches only <prefix>/lib, and because no shared libraries are
# there the linker takes libssl.a and libcrypto.a, so binaries, wheels and Node
# addons do not need OpenSSL installed at runtime.
#
# Usage: scripts/ci/static-openssl.sh <prefix> [version]

set -euo pipefail

prefix="${1:?usage: static-openssl.sh <prefix> [version]}"
version="${2:-${OPENSSL_VERSION:-3.0.15}}"

if [[ -f "$prefix/lib/libssl.a" && -f "$prefix/lib/libcrypto.a" ]]; then
  echo "static OpenSSL already in $prefix"
  exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl -fsSL "https://github.com/openssl/openssl/releases/download/openssl-$version/openssl-$version.tar.gz" |
  tar -xz -C "$work"
cd "$work/openssl-$version"

# -fPIC: the static archives are linked into shared objects (wheels, addons).
./Configure no-shared no-tests -fPIC --prefix="$prefix" --libdir=lib >/dev/null
make -j"$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 4)" >/dev/null
make install_sw >/dev/null

if compgen -G "$prefix/lib/*.so*" >/dev/null || compgen -G "$prefix/lib/*.dylib" >/dev/null; then
  echo "error: shared OpenSSL libraries were installed into $prefix/lib" >&2
  exit 1
fi
echo "static OpenSSL $version installed in $prefix"
