#!/usr/bin/env bash
set -euo pipefail

repo_dir="/repo"
out_dir="/out/nix"
version="${WPRS_VERSION:?WPRS_VERSION is required}"

rm -rf "$out_dir"
mkdir -p "$out_dir"

cp "$repo_dir/package/nix/flake.nix" "$out_dir/flake.nix"
sed -i -e "s/@VERSION@/$version/g" "$out_dir/flake.nix"

