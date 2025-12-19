#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

usage() {
  cat <<'EOF'
Usage: ./scripts/package.sh [--targets "t1 t2 ..."] [--bins "b1 b2 ..."] [--formats "f1 f2 ..."]

Builds release binaries and packages per-target artifacts under ./dist/.

Defaults:
  targets:  host + common Linux targets
  bins:     wprsc (always), and wprsd on Linux targets
  formats:  tar.gz tar.xz deb rpm pip arch nix brew

Formats:
  - tar.gz, tar.xz: portable archives (per target)
  - deb, rpm: distro-native packages via Dockerized tools from distro repos (Linux targets only)
  - pip: Python wheel for the `wprs` launcher (no Rust binaries)
  - arch, nix, brew: emits packaging templates under dist/packaging/

Notes:
  - Linux targets use `cross` when available; otherwise falls back to `cargo`.
  - Non-Linux targets are built with `cargo` (must run on that OS/toolchain).
  - deb/rpm require `docker`/`podman` (via the `docker` CLI).
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

targets_override=""
bins_override=""
formats_override=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --targets)
      targets_override="$2"
      shift 2
      ;;
    --bins)
      bins_override="$2"
      shift 2
      ;;
    --formats)
      formats_override="$2"
      shift 2
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

host_target="$(rustc -vV | awk '/^host:/{print $2}')"
if [[ -z "$host_target" ]]; then
  echo "unable to determine host target (rustc -vV)" >&2
  exit 1
fi

version="$(awk -F'"' '/^version[[:space:]]*=[[:space:]]*"/{print $2; exit}' Cargo.toml)"
if [[ -z "$version" ]]; then
  echo "unable to parse version from Cargo.toml" >&2
  exit 1
fi

default_targets=(
  "$host_target"
  x86_64-unknown-linux-gnu
  aarch64-unknown-linux-gnu
)

if [[ -n "$targets_override" ]]; then
  # shellcheck disable=SC2206
  targets=( $targets_override )
else
  targets=("${default_targets[@]}")
fi

default_formats=(tar.gz tar.xz deb rpm pip arch nix brew)
if [[ -n "$formats_override" ]]; then
  # shellcheck disable=SC2206
  formats=( $formats_override )
else
  formats=("${default_formats[@]}")
fi

dist_dir="dist"
rm -rf "$dist_dir"
mkdir -p "$dist_dir"

have_cross=0
if command -v cross >/dev/null 2>&1; then
  if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
    have_cross=1
  else
    echo "note: 'cross' is installed but Docker isn't available; skipping non-host Linux targets" >&2
  fi
fi

want_format() {
  local f="$1"
  for v in "${formats[@]}"; do
    if [[ "$v" == "$f" ]]; then
      return 0
    fi
  done
  return 1
}

require_tool() {
  local t="$1"
  local hint="$2"
  if ! command -v "$t" >/dev/null 2>&1; then
    echo "missing required tool: $t" >&2
    if [[ -n "$hint" ]]; then
      echo "$hint" >&2
    fi
    return 1
  fi
}

have_docker=0
if command -v docker >/dev/null 2>&1; then
  have_docker=1
fi

ensure_packager_image() {
  local tag="$1"
  local dockerfile="$2"
  if [[ $have_docker -eq 0 ]]; then
    echo "missing required tool: docker" >&2
    echo "Hint: install podman/docker and ensure the 'docker' CLI is available" >&2
    return 1
  fi
  echo "+ docker build -f $dockerfile -t $tag ." >&2
  docker build -f "$dockerfile" -t "$tag" .
}

package_deb_from_staging() {
  local target="$1"
  local staging="$2"

  local deb_arch
  deb_arch="$(deb_arch_from_target "$target")"
  if [[ -z "$deb_arch" ]]; then
    echo "skipping deb for unsupported target: $target" >&2
    return 0
  fi

  local out_deb="$dist_dir/wprs_${version}_${deb_arch}.deb"
  ensure_packager_image "wprs-packager-deb:latest" "package/deb/Dockerfile" || return 0

  echo "+ docker run (deb) -> $out_deb" >&2
  docker run --rm \
    -e "WPRS_VERSION=$version" \
    -e "WPRS_ARCH=$deb_arch" \
    -e "WPRS_OUT=/out/$(basename "$out_deb")" \
    -v "$(pwd)/$staging:/staging:ro" \
    -v "$(pwd)/$dist_dir:/out" \
    wprs-packager-deb:latest
}

package_rpm_from_staging() {
  local target="$1"
  local staging="$2"

  local rpm_arch
  rpm_arch="$(rpm_arch_from_target "$target")"
  if [[ -z "$rpm_arch" ]]; then
    echo "skipping rpm for unsupported target: $target" >&2
    return 0
  fi

  local out_rpm="$dist_dir/wprs-${version}-1.${rpm_arch}.rpm"
  ensure_packager_image "wprs-packager-rpm:latest" "package/rpm/Dockerfile" || return 0

  echo "+ docker run (rpm) -> $out_rpm" >&2
  docker run --rm \
    -e "WPRS_VERSION=$version" \
    -e "WPRS_ARCH=$rpm_arch" \
    -e "WPRS_OUT=/out/$(basename "$out_rpm")" \
    -v "$(pwd)/$staging:/staging:ro" \
    -v "$(pwd)/$dist_dir:/out" \
    wprs-packager-rpm:latest
}

build_one() {
  local target="$1"
  local bin="$2"
  local features="$3"

  local tool=(cargo)
  if [[ "$target" == *"unknown-linux"* && $have_cross -eq 1 ]]; then
    tool=(cross)
  fi

  if [[ "${tool[0]}" == "cargo" && "$target" != "$host_target" ]]; then
    echo "skipping $bin for target $target (requires cross+Docker or a host toolchain for that target)" >&2
    return 2
  fi

  local args=(build --profile=release-lto --target "$target" --bin "$bin")
  if [[ -n "$features" ]]; then
    args+=(--features "$features")
  fi

  echo "+ ${tool[*]} ${args[*]}" >&2
  "${tool[@]}" "${args[@]}"
}

bin_path() {
  local target="$1"
  local bin="$2"
  local exe="$bin"
  if [[ "$target" == *"windows"* ]]; then
    exe+='.exe'
  fi
  echo "target/$target/release-lto/$exe"
}

deb_arch_from_target() {
  case "$1" in
    x86_64-*) echo amd64 ;;
    aarch64-*) echo arm64 ;;
    *) echo "" ;;
  esac
}

rpm_arch_from_target() {
  case "$1" in
    x86_64-*) echo x86_64 ;;
    aarch64-*) echo aarch64 ;;
    *) echo "" ;;
  esac
}

write_packaging_templates() {
  if ! (want_format arch || want_format brew || want_format nix || want_format pip); then
    return 0
  fi

  local out_dir="$dist_dir/packaging"
  rm -rf "$out_dir"
  mkdir -p "$out_dir"

  # Prefer running template rendering in small Docker/Podman containers so the
  # host doesn't need extra tools beyond the 'docker' CLI.
  if [[ $have_docker -eq 1 ]]; then
    if want_format arch; then
      ensure_packager_image "wprs-packager-arch-template:latest" "package/arch/Dockerfile" || true
      echo "+ docker run (arch template)" >&2
      docker run --rm \
        -e "WPRS_VERSION=$version" \
        -v "$(pwd):/repo:ro" \
        -v "$(pwd)/$out_dir:/out" \
        wprs-packager-arch-template:latest
    fi

    if want_format brew; then
      ensure_packager_image "wprs-packager-brew-template:latest" "package/brew/Dockerfile" || true
      echo "+ docker run (brew template)" >&2
      docker run --rm \
        -e "WPRS_VERSION=$version" \
        -v "$(pwd):/repo:ro" \
        -v "$(pwd)/$out_dir:/out" \
        wprs-packager-brew-template:latest
    fi

    if want_format nix; then
      ensure_packager_image "wprs-packager-nix-template:latest" "package/nix/Dockerfile" || true
      echo "+ docker run (nix template)" >&2
      docker run --rm \
        -e "WPRS_VERSION=$version" \
        -v "$(pwd):/repo:ro" \
        -v "$(pwd)/$out_dir:/out" \
        wprs-packager-nix-template:latest
    fi

    if want_format pip; then
      mkdir -p "$out_dir/python"
      cp -R package/python/* "$out_dir/python/"
    fi

    return 0
  fi

  # Fallback: render templates directly on the host.
  # These templates require you to fill in release URLs and SHA256s.
  if want_format arch; then
    mkdir -p "$out_dir/arch"
    cp package/arch/PKGBUILD.in "$out_dir/arch/PKGBUILD.in"
  fi
  if want_format brew; then
    mkdir -p "$out_dir/brew"
    cp package/brew/wprs.rb.in "$out_dir/brew/wprs.rb.in"
  fi
  if want_format nix; then
    mkdir -p "$out_dir/nix"
    cp package/nix/flake.nix "$out_dir/nix/flake.nix"
  fi
  if want_format pip; then
    mkdir -p "$out_dir/python"
    cp -R package/python/* "$out_dir/python/"
  fi

  find "$out_dir" -type f -name '*.in' -print0 | while IFS= read -r -d '' f; do
    sed -e "s/@VERSION@/$version/g" "$f" > "${f%.in}"
  done
}

build_pip_wheel() {
  if ! want_format pip; then
    return 0
  fi

  require_tool python3 "Install Python 3.9+" >&2 || return 1

  local py_out="$dist_dir/pip"
  rm -rf "$py_out"
  mkdir -p "$py_out"

  local tmp="$dist_dir/.tmp-python"
  rm -rf "$tmp"
  mkdir -p "$tmp"
  cp -R package/python/* "$tmp/"
  cp README.md "$tmp/README.md"
  cp wprs "$tmp/wprs/wprs.py"

  perl -0777 -i -pe "s/^version\s*=\s*\"0\.0\.0\"/version = \"$version\"/m" "$tmp/pyproject.toml"
  perl -0777 -i -pe "s/__version__\s*=\s*\"0\.0\.0\"/__version__ = \"$version\"/m" "$tmp/wprs/__init__.py"

  if python3 -c "import build" >/dev/null 2>&1; then
    # Prefer the host environment to avoid requiring network access to install
    # build-system dependencies (common in sandboxed CI environments).
    echo "+ python3 -m build --no-isolation" >&2
    if ! (cd "$tmp" && python3 -m build --no-isolation --outdir "../pip"); then
      echo "note: pip build failed with --no-isolation; retrying with isolation" >&2
      echo "+ python3 -m build" >&2
      (cd "$tmp" && python3 -m build --outdir "../pip")
    fi
  else
    echo "python module 'build' not installed; skipping pip wheel" >&2
    echo "Hint: python3 -m pip install --user build" >&2
    return 1
  fi
}

package_target() {
  local target="$1"

  if [[ "$target" != "$host_target" && "$target" == *"unknown-linux"* && $have_cross -eq 0 ]]; then
    echo "skipping target $target (cross+Docker not available)" >&2
    return 0
  fi

  local staging="$dist_dir/stage-$target"
  rm -rf "$staging"
  mkdir -p "$staging/bin" "$staging/share/doc/wprs" "$staging/lib/systemd/user"

  cp LICENSE "$staging/share/doc/wprs/"
  cp README.md "$staging/share/doc/wprs/"

  local bins
  if [[ -n "$bins_override" ]]; then
    bins="$bins_override"
  else
    bins="wprsc"
    if [[ "$target" == *"unknown-linux"* ]]; then
      bins+=" wprsd"
    fi
  fi

  local built_any=0
  for bin in $bins; do
    local features=""
    if [[ "$bin" == "wprsd" ]]; then
      features="server,wayland-client"
    fi
    if ! build_one "$target" "$bin" "$features"; then
      echo "skipping bin $bin for target $target due to build failure" >&2
      continue
    fi

    local path
    path="$(bin_path "$target" "$bin")"
    if [[ ! -f "$path" ]]; then
      echo "expected binary not found: $path" >&2
      exit 1
    fi
    cp "$path" "$staging/bin/"
    built_any=1
  done

  if [[ "$target" == *"unknown-linux"* ]]; then
    if [[ -f package/wprsd.service && -f "$staging/bin/wprsd" ]]; then
      cp package/wprsd.service "$staging/lib/systemd/user/wprsd.service"
    fi
  fi

  if [[ $built_any -eq 0 ]]; then
    echo "skipping packaging for target $target (no bins built)" >&2
    return 0
  fi

  if want_format tar.gz; then
    local out_gz="$dist_dir/wprs-$version-$target.tar.gz"
    echo "+ tar -czf $out_gz" >&2
    tar -C "$staging" -czf "$out_gz" .
  fi

  if want_format tar.xz; then
    local out_xz="$dist_dir/wprs-$version-$target.tar.xz"
    echo "+ tar -cJf $out_xz" >&2
    tar -C "$staging" -cJf "$out_xz" .
  fi

  if [[ "$target" == *"unknown-linux"* ]]; then
    if want_format deb; then
      package_deb_from_staging "$target" "$staging"
    fi

    if want_format rpm; then
      package_rpm_from_staging "$target" "$staging"
    fi
  fi
}

write_packaging_templates
build_pip_wheel || true

for target in "${targets[@]}"; do
  package_target "$target"
done

echo "Artifacts written to $dist_dir/" >&2
