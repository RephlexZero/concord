#!/usr/bin/env bash
# Put a concord binary on PATH: a checksum-verified release download when the
# action is used at a release tag, otherwise a build of the action's source.
set -euo pipefail

ref="${VERSION:-${ACTION_REF:-}}"
repo="${ACTION_REPO:-}"
bin="$RUNNER_TEMP/concord-bin"
mkdir -p "$bin"

case "$RUNNER_OS-$RUNNER_ARCH" in
  Linux-X64) target=x86_64-unknown-linux-musl ;;
  Linux-ARM64) target=aarch64-unknown-linux-musl ;;
  macOS-ARM64) target=aarch64-apple-darwin ;;
  macOS-X64) target=x86_64-apple-darwin ;;
  Windows-X64) target=x86_64-pc-windows-msvc ;;
  *) target="" ;;
esac

sha256_check() {
  if command -v sha256sum >/dev/null; then sha256sum -c "$1"; else shasum -a 256 -c "$1"; fi
}

download() {
  local base="$1" name="concord-$target.tar.gz"
  echo "Downloading $base/$name"
  curl -fsSL --retry 3 -o "$RUNNER_TEMP/$name" "$base/$name" || return 1
  curl -fsSL --retry 3 -o "$RUNNER_TEMP/$name.sha256" "$base/$name.sha256" || return 1
  if ! (cd "$RUNNER_TEMP" && sha256_check "$name.sha256"); then
    echo "::error::checksum mismatch for $name; refusing to run it"
    exit 1
  fi
  tar -xzf "$RUNNER_TEMP/$name" -C "$bin"
}

build() {
  echo "Building concord from $GITHUB_ACTION_PATH"
  cargo build --release --locked \
    --manifest-path "$GITHUB_ACTION_PATH/Cargo.toml" \
    --target-dir "$RUNNER_TEMP/concord-target"
  local exe=concord
  [ "$RUNNER_OS" = Windows ] && exe=concord.exe
  cp "$RUNNER_TEMP/concord-target/release/$exe" "$bin/"
}

base=""
if [ -n "$repo" ] && [ -n "$target" ]; then
  if [[ "$ref" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    base="https://github.com/$repo/releases/download/$ref"
  elif [[ "$ref" =~ ^v[0-9]+$ ]]; then
    base="https://github.com/$repo/releases/latest/download"
  fi
fi

if [ -n "$base" ] && download "$base"; then
  :
else
  [ -n "$base" ] && echo "::warning::no concord release at $base for $target; building from source"
  build
fi

echo "$bin" >> "$GITHUB_PATH"
"$bin/concord" --version
