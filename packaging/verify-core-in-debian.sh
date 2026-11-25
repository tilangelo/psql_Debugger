#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
: "${PGDEBUG_GUI_TEST_PORT:?Run through verify-gui-in-debian.sh}"
: "${PGDEBUG_GUI_TEST_PASSWORD:?Fixture password is required}"
export PGDEBUG_DATABASE_URL="host=127.0.0.1 port=$PGDEBUG_GUI_TEST_PORT user=debugger_test dbname=gui_plugin_test password=$PGDEBUG_GUI_TEST_PASSWORD"
export PGDEBUG_PG_HOME=/usr/lib/postgresql/17
cargo build --release --locked --bin pg-debugger
cargo test --release --locked --lib
binary="${CARGO_TARGET_DIR:-target}/release/pg-debugger"
work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT

status=0
timeout 12 "$binary" interactive-run 'public.debug_smoke(integer)' 'SELECT public.core_missing()' >"$work/error.log" 2>&1 || status=$?
if [[ "$status" != 1 ]] || ! grep -q '42883' "$work/error.log"; then cat "$work/error.log"; echo "Unexpected early-error exit: $status"; exit 1; fi
status=0
timeout 12 "$binary" interactive-run 'public.debug_smoke(integer)' 'SELECT 1' >"$work/no-break.log" 2>&1 || status=$?
if [[ "$status" != 1 ]] || ! grep -q 'SQL завершился до первой остановки' "$work/no-break.log"; then cat "$work/no-break.log"; exit 1; fi
printf 'c\n' | timeout 12 "$binary" interactive-run 'public.debug_smoke(integer)' 'SELECT public.debug_smoke(3)' >"$work/success.log" 2>&1
grep -q 'Target SQL succeeded' "$work/success.log"

version="$(/usr/lib/postgresql/17/bin/psql -h 127.0.0.1 -p "$PGDEBUG_GUI_TEST_PORT" -U debugger_test -d gui_plugin_test -XAtc "SELECT current_setting('server_version_num')")"
"$binary" info >"$work/info.log"
grep -q "Server version number: $version" "$work/info.log"
echo "CLI early failure, missing breakpoint, successful call and runtime version passed (server $version)."

if [[ "$EUID" == 0 ]]; then
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='runuser -u postgres --' cargo test --release --locked --test demo_lifecycle -- --ignored
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER='runuser -u postgres --' cargo test --manifest-path gui/Cargo.toml --release --locked --no-default-features --features local-server-tests --test core_disconnect -- --ignored
else
    cargo test --release --locked --test demo_lifecycle -- --ignored
    cargo test --manifest-path gui/Cargo.toml --release --locked --no-default-features --features local-server-tests --test core_disconnect -- --ignored
fi
