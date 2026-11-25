#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
: "${PGDEBUG_GUI_TEST_PORT:?Run through verify-gui-in-debian.sh}"
: "${PGDEBUG_GUI_TEST_PASSWORD:?Fixture password required}"
python3 packaging/check-editor.py
cargo test --manifest-path web/server/Cargo.toml --release --locked
binary="${PGDEBUG_WEB_TEST_BINARY:-}"
if [[ -z "$binary" ]]; then
    cargo build --manifest-path web/server/Cargo.toml --release --locked
    binary="${CARGO_TARGET_DIR:-web/server/target}/release/pg-debugger-web"
fi
binary="$(realpath -- "$binary")"
test -x "$binary"
python3 web/server/tests/startup_cli.py "$binary"
python3 web/server/tests/isolated_demo_http.py "$binary"
port="${PGDEBUG_WEB_TEST_PORT:-18766}"
work="$(mktemp -d)"
"$binary" --port "$port" --no-browser >"$work/web.log" 2>&1 &
web_pid=$!
cleanup() { kill "$web_pid" 2>/dev/null || true; wait "$web_pid" 2>/dev/null || true; rm -rf -- "$work"; }
trap cleanup EXIT
export PGDEBUG_WEB_TEST_PID="$web_pid"
export PGDEBUG_WEB_TEST_URL="http://127.0.0.1:$port"
python3 web/server/tests/catalog_http.py
python3 web/server/tests/editor_assets_http.py

python3 web/server/tests/apply_http.py

python3 web/server/tests/metadata_http.py

python3 web/server/tests/enrichment_http.py

python3 web/server/tests/checkpoint_http.py

python3 web/server/tests/onboarding_http.py

python3 web/server/tests/body_http.py

python3 web/server/tests/scalar_types_http.py

python3 web/server/tests/shop_http.py
python3 web/server/tests/debug_http.py
