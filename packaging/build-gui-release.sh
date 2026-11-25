#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
if [[ "$(uname -s)" != Linux ]]; then
    echo 'Соберите GUI в Debian 13.' >&2
    exit 1
fi

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' gui/Cargo.toml | head -n 1)"
architecture="$(dpkg --print-architecture)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-"$root/gui/target"}"
name="pg-debugger-gui-$version-debian13-$architecture"
archive="$root/dist/linux/${name}.tar.gz"
stage="$(mktemp -d)"
trap 'rm -rf -- "$stage"' EXIT
mkdir -p "$stage/$name" "$root/dist/linux"

cargo build --release --locked --manifest-path gui/Cargo.toml --bin pg-debugger-gui
install -m755 "$CARGO_TARGET_DIR/release/pg-debugger-gui" "$stage/$name/pg-debugger-gui"
install -m644 debug_smoke.sql debug_failure.sql gui/tests/debug_fixture.sql "$stage/$name/"
install -d "$stage/$name/examples/shop"
install -m644 examples/shop/*.sql "$stage/$name/examples/shop/"
tar -C "$stage" -czf "$archive" "$name"
(
    cd "$(dirname -- "$archive")"
    sha256sum "$(basename -- "$archive")" > "$(basename -- "$archive").sha256"
)
printf '%s\n%s\n' "$archive" "$archive.sha256"
