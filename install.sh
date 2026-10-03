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
# Where cargo put it: `CARGO_TARGET_DIR` or `build.target-dir` may move it away from `target/`.
target=$(cargo metadata --format-version 1 --no-deps --manifest-path "$dir/Cargo.toml" |
  sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
target=${target:-$dir/target}
mkdir -p "$bin"
tmp="$bin/.noble-dev.new"
if ! install -m 755 "$target/release/noble" "$tmp" || ! mv -f "$tmp" "$bin/noble-dev"; then
  rm -f "$tmp"
  echo "NOBLE install failed." >&2
  exit 1
fi
echo "Installed: $("$bin/noble-dev" --version)  (run: noble-dev)"
