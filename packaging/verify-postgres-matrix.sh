#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
if (( $# == 0 )); then set -- 17 18; fi
for major; do
    [[ "$major" =~ ^[0-9]+$ ]] || { echo "Укажите номер версии PostgreSQL. Получено: $major" >&2; exit 2; }
    home="/usr/lib/postgresql/$major"
    [[ -x "$home/bin/postgres" && -x "$home/bin/psql" && -f "$home/lib/plugin_debugger.so" ]] || {
        echo "Подготовьте postgresql-$major, postgresql-client-$major и postgresql-$major-pldebugger." >&2; exit 2;
    }
    echo "Проверка веб-отладчика: $("$home/bin/postgres" --version)"
    PGDEBUG_PG_HOME="$home" bash packaging/verify-gui-in-debian.sh --web-only
done
