#!/bin/sh
# Builds the working tree and installs it as `noble-dev`, next to (never over)
# a stable `noble` from `cargo install noble` or a release. The Linux/macOS
# counterpart of install.cmd. Works while noble-dev is running: the new binary is
# moved into place with a rename, the running one keeps its old file.
set -eu
dir=$(cd "$(dirname "$0")" && pwd)
bin="${CARGO_HOME:-$HOME/.cargo}/bin"
if ! cargo build --release --quiet --manifest-path "$dir/Cargo.toml"; then
  echo "NOBLE build failed." >&2
  exit 1
fi
mkdir -p "$bin"
tmp="$bin/.noble-dev.new"
if ! install -m 755 "$dir/target/release/noble" "$tmp" || ! mv -f "$tmp" "$bin/noble-dev"; then
  rm -f "$tmp"
  echo "NOBLE install failed." >&2
  exit 1
fi
echo "Installed: $("$bin/noble-dev" --version)  (run: noble-dev)"
