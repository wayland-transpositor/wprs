#!/usr/bin/env bash
set -euo pipefail

staging_dir="/staging"

version="${WPRS_VERSION:?WPRS_VERSION is required}"
arch="${WPRS_ARCH:?WPRS_ARCH is required}"
out="${WPRS_OUT:?WPRS_OUT is required}"

tmp_root="$(mktemp -d)"
trap 'rm -rf "$tmp_root"' EXIT

topdir="$tmp_root/rpmbuild"
mkdir -p "$topdir/BUILD" "$topdir/BUILDROOT" "$topdir/RPMS" "$topdir/SOURCES" "$topdir/SPECS" "$topdir/SRPMS"

srcdir="$tmp_root/src/wprs-${version}"
mkdir -p "$srcdir/usr"

if [[ -d "$staging_dir/bin" ]]; then
  mkdir -p "$srcdir/usr/bin"
  cp -a "$staging_dir/bin/." "$srcdir/usr/bin/"
fi

if [[ -d "$staging_dir/share" ]]; then
  mkdir -p "$srcdir/usr/share"
  cp -a "$staging_dir/share/." "$srcdir/usr/share/"
fi

if [[ -d "$staging_dir/lib" ]]; then
  mkdir -p "$srcdir/usr/lib"
  cp -a "$staging_dir/lib/." "$srcdir/usr/lib/"
fi

tar -C "$(dirname "$srcdir")" -czf "$topdir/SOURCES/wprs-${version}.tar.gz" "$(basename "$srcdir")"

files_list=(
  /usr/bin/wprsc
  /usr/share/doc/wprs/*
)
if [[ -f "$srcdir/usr/bin/wprsd" ]]; then
  files_list+=(/usr/bin/wprsd)
fi
if [[ -f "$srcdir/usr/lib/systemd/user/wprsd.service" ]]; then
  files_list+=(/usr/lib/systemd/user/wprsd.service)
fi

changelog_date="$(date -u '+%a %b %d %Y')"

cat >"$topdir/SPECS/wprs.spec" <<EOF
%global debug_package %{nil}
%global __os_install_post %{nil}

Name: wprs
Version: ${version}
Release: 1%{?dist}
Summary: wprs (wprsc client + wprsd server)
License: Apache-2.0
URL: https://github.com/google/wprs
Source0: wprs-%{version}.tar.gz

ExclusiveArch: ${arch}

%description
wprs is a Rust-based remote Wayland protocol bridge.

%prep
%setup -q

%install
mkdir -p %{buildroot}/
cp -a usr %{buildroot}/usr

%files
$(printf '%s\n' "${files_list[@]}")

%changelog
* ${changelog_date} wprs <wprs> - %{version}-1
- Packaged by package/rpm/build.sh
EOF

rpmbuild \
  --define "_topdir $topdir" \
  --define "_build_id_links none" \
  -bb "$topdir/SPECS/wprs.spec" >/dev/null

mkdir -p "$(dirname "$out")"
found_rpm="$(find "$topdir/RPMS" -type f -name '*.rpm' | head -n 1)"
if [[ -z "$found_rpm" ]]; then
  echo "rpmbuild did not produce an rpm" >&2
  exit 1
fi

cp -f "$found_rpm" "$out"
