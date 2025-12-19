#!/usr/bin/env bash
set -euo pipefail

repo_dir="/repo"
out_dir="/out/arch"
version="${WPRS_VERSION:?WPRS_VERSION is required}"

rm -rf "$out_dir"
mkdir -p "$out_dir"

cp "$repo_dir/package/arch/PKGBUILD.in" "$out_dir/PKGBUILD.in"
sed -e "s/@VERSION@/$version/g" "$out_dir/PKGBUILD.in" > "$out_dir/PKGBUILD"

