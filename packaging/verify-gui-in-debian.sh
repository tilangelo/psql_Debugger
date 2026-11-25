#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
case "${1:-}" in
    ""|--web-only) ;;
    *) echo 'Запуск: bash packaging/verify-gui-in-debian.sh [--web-only]' >&2; exit 2 ;;
esac
export PGDEBUG_PG_HOME="${PGDEBUG_PG_HOME:-/usr/lib/postgresql/17}"
pg="$PGDEBUG_PG_HOME/bin"
export PGDEBUG_TEST_PSQL="$pg/psql"
if [[ ! -x "$pg/postgres" || ! -x "$pg/psql" || ! -f "$PGDEBUG_PG_HOME/lib/plugin_debugger.so" ]]; then
    echo "Нужны PostgreSQL, psql и plugin_debugger в $PGDEBUG_PG_HOME." >&2
    exit 2
fi
work="$(mktemp -d)"
data="$work/data"
port="${PGDEBUG_GUI_VERIFY_PORT:-15483}"
password="$(head -c 24 /dev/urandom | base64)"

run_pg() {
    if [[ "$EUID" == 0 ]]; then
        runuser -u postgres -- "$@"
    else
        "$@"
    fi
}

cleanup() {
    if [[ -f "$data/postmaster.pid" ]]; then
        run_pg "$pg/pg_ctl" -D "$data" -m fast -w stop
    fi
    rm -rf -- "$work"
}
trap cleanup EXIT
printf '%s\n' "$password" > "$work/password"
chmod 600 "$work/password"
if [[ "$EUID" == 0 ]]; then chown -R postgres:postgres "$work"; fi

run_pg "$pg/initdb" -D "$data" -U debugger_test --auth-host=scram-sha-256 \
    --auth-local=trust --pwfile="$work/password" --locale=C --encoding=UTF8 --no-instructions >/dev/null
rm -- "$work/password"
run_pg "$pg/pg_ctl" -D "$data" -l "$work/server.log" -o \
    "-p $port -c listen_addresses=127.0.0.1 -c unix_socket_directories= -c shared_preload_libraries=plugin_debugger" -w start

export PGPASSWORD="$password"
psql=("$pg/psql" -h 127.0.0.1 -p "$port" -U debugger_test -w -X -v ON_ERROR_STOP=1)
"${psql[@]}" -d postgres -c 'CREATE DATABASE gui_catalog_test' -c 'CREATE DATABASE gui_plugin_test' -c 'CREATE DATABASE gui_denied_test' -c 'CREATE DATABASE gui_setup_test'
"${psql[@]}" -d gui_catalog_test -f gui/tests/catalog_fixture.sql
"${psql[@]}" -d gui_plugin_test -c 'CREATE EXTENSION pldbgapi' \
    -f debug_smoke.sql -f debug_failure.sql -f gui/tests/debug_fixture.sql
"${psql[@]}" -d gui_plugin_test <<'SQL'
\getenv fixture_password PGPASSWORD
CREATE ROLE gui_reader LOGIN PASSWORD :'fixture_password';
CREATE FUNCTION public.reader_owned() RETURNS integer LANGUAGE plpgsql AS $$ BEGIN RETURN 1; END; $$;
ALTER FUNCTION public.reader_owned() OWNER TO gui_reader;
SQL
"${psql[@]}" -d gui_denied_test -c 'CREATE EXTENSION pldbgapi' \
    -c 'REVOKE EXECUTE ON FUNCTION public.pldbg_oid_debug(oid) FROM PUBLIC'
"${psql[@]}" -d postgres -c 'CREATE DATABASE gui_shop_test'
shop_dir="${PGDEBUG_WEB_SHOP_DIR:-examples/shop}"
"${psql[@]}" -d gui_shop_test -f "$shop_dir/schema.sql" -f "$shop_dir/metadata.sql"
if [[ -n "${PGDEBUG_WEB_SHOP_DIR:-}" ]]; then
    "${psql[@]}" -d gui_shop_test -f "$shop_dir/check.sql"
fi
export PGDEBUG_GUI_TEST_PORT="$port" PGDEBUG_GUI_TEST_PASSWORD="$password"
if [[ "${1:-}" == --web-only ]]; then
    bash packaging/verify-web-in-debian.sh
    exit
fi
cargo test --manifest-path core/Cargo.toml --release --locked
cargo test --manifest-path gui/Cargo.toml --release --locked --no-default-features -- --include-ignored
runner='xvfb-run -a env GTK_A11Y=none GDK_BACKEND=x11 GSK_RENDERER=cairo GSETTINGS_BACKEND=memory'
if [[ "$EUID" == 0 ]]; then runner="runuser -u postgres -- $runner"; fi
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER="$runner" \
    cargo test --manifest-path gui/Cargo.toml --release --locked --bin pg-debugger-gui -- --ignored

bash packaging/verify-core-in-debian.sh

if [[ -f web/server/Cargo.toml ]]; then bash packaging/verify-web-in-debian.sh; fi
