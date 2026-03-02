#!/usr/bin/env bash
set -euo pipefail

repo_dir="/repo"
out_dir="/out/brew"
version="${WPRS_VERSION:?WPRS_VERSION is required}"

rm -rf "$out_dir"
mkdir -p "$out_dir"

cp "$repo_dir/package/brew/wprs.rb.in" "$out_dir/wprs.rb.in"
sed -e "s/@VERSION@/$version/g" "$out_dir/wprs.rb.in" > "$out_dir/wprs.rb"

