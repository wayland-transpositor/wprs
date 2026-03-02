#!/usr/bin/env bash
set -euo pipefail

staging_dir="/staging"

version="${WPRS_VERSION:?WPRS_VERSION is required}"
arch="${WPRS_ARCH:?WPRS_ARCH is required}"
out="${WPRS_OUT:?WPRS_OUT is required}"

tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

pkg_root="$tmp_root/root"
mkdir -p "$pkg_root/DEBIAN" "$pkg_root/usr"

if [[ -d "$staging_dir/bin" ]]; then
  mkdir -p "$pkg_root/usr/bin"
  cp -a "$staging_dir/bin/." "$pkg_root/usr/bin/"
fi

if [[ -d "$staging_dir/share" ]]; then
  mkdir -p "$pkg_root/usr/share"
  cp -a "$staging_dir/share/." "$pkg_root/usr/share/"
fi

if [[ -d "$staging_dir/lib" ]]; then
  mkdir -p "$pkg_root/usr/lib"
  cp -a "$staging_dir/lib/." "$pkg_root/usr/lib/"
fi

installed_size_kb="$(du -sk "$pkg_root/usr" | awk '{print $1}')"

cat >"$pkg_root/DEBIAN/control" <<EOF
Package: wprs
Version: ${version}
Section: utils
Priority: optional
Architecture: ${arch}
Maintainer: wprs
Installed-Size: ${installed_size_kb}
Description: wprs (wprsc client + wprsd server)
 wprs is a Rust-based remote Wayland protocol bridge.
EOF

mkdir -p "$(dirname "$out")"
dpkg-deb --build --root-owner-group "$pkg_root" "$out" >/dev/null

