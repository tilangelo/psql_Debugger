#!/usr/bin/env bash
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
[[ "$(uname -s)" == Linux ]] || { echo 'Соберите Linux-выпуск в Debian 13.' >&2; exit 1; }
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' web/server/Cargo.toml | head -n 1)"
architecture="$(dpkg --print-architecture)"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-"$root/web/server/target"}"
name="pg-debugger-web-$version-debian13-$architecture"
archive="$root/dist/web/$name.tar.gz"
stage="$(mktemp -d)"
trap 'rm -rf -- "$stage"' EXIT
mkdir -p "$stage/$name" "$root/dist/web"
python3 packaging/check-editor.py
cargo build --release --locked --manifest-path web/server/Cargo.toml
install -m755 "$CARGO_TARGET_DIR/release/pg-debugger-web" "$stage/$name/pg-debugger-web"
install -m644 README.md "$stage/$name/README.md"
install -m644 web/prototype/vendor/THIRD_PARTY_LICENSES.txt "$stage/$name/THIRD_PARTY_LICENSES.txt"
mkdir -p "$stage/$name"
tar -C "$stage" -czf "$archive" "$name"
(cd "$(dirname -- "$archive")"; sha256sum "$(basename -- "$archive")" > "$(basename -- "$archive").sha256")
printf '%s\n%s\n' "$archive" "$archive.sha256"
