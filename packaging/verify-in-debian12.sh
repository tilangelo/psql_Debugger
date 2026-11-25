#!/bin/sh
set -eu

# Запускать внутри временного контейнера debian:bookworm-slim с пакетом в /release.
export DEBIAN_FRONTEND=noninteractive
if ! dpkg-query -W -f='${Status}' postgresql-15-pldebugger 2>/dev/null | grep -Fq 'install ok installed'; then
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends postgresql-15-pldebugger
fi
dpkg -i /release/pg-debugger_0.1.0-1_amd64.deb

pg-debugger help | grep -F 'interactive-line TARGET LINE'

if ! id -u pgdebugtest >/dev/null 2>&1; then
    useradd -m -s /bin/sh pgdebugtest
fi
if output="$(printf '\n\n0\n' | timeout 120 runuser -u pgdebugtest -- pg-debugger 2>&1)"; then
    :
else
    printf '%s\n' "$output" >&2
    log=/home/pgdebugtest/.local/share/pg-debugger/demo-pg15-0.1.0.log
    if [ -f "$log" ]; then
        printf '\nЖурнал PostgreSQL:\n' >&2
        cat "$log" >&2
    fi
    exit 1
fi
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -F 'Target PID:'
printf '%s\n' "$output" | grep -F 'debug_smoke'
printf '%s\n' "$output" | grep -E '^[[:space:]]*8[[:space:]]*$'
printf '%s\n' "$output" | grep -F 'Готово.'

echo 'Debian 12: установка пакета и демонстрация прошли.'
