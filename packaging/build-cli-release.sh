#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
if [[ "$(uname -s)" != Linux ]]; then
    echo 'Соберите Linux CLI в Debian 13.' >&2
    exit 1
fi
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
architecture="$(dpkg --print-architecture)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-"$root/target"}"
name="pg-debugger-cli-$version-debian13-$architecture"
archive="$root/dist/linux/$name.tar.gz"
stage="$(mktemp -d)"
trap 'rm -rf -- "$stage"' EXIT
mkdir -p "$stage/$name" "$root/dist/linux"
cargo build --release --locked --bin pg-debugger
install -m755 "$CARGO_TARGET_DIR/release/pg-debugger" "$stage/$name/pg-debugger"
install -m644 debug_smoke.sql debug_failure.sql "$stage/$name/"
tar -C "$stage" -czf "$archive" "$name"
(
    cd "$(dirname -- "$archive")"
    sha256sum "$(basename -- "$archive")" > "$(basename -- "$archive").sha256"
)
printf '%s\n%s\n' "$archive" "$archive.sha256"
