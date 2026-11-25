#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"

if [[ "$(uname -s)" != Linux ]]; then
    echo 'Сборку Debian-пакета нужно запускать в Linux.' >&2
    exit 1
fi

for tool in cargo dpkg-deb install tar sha256sum; do
    command -v "$tool" >/dev/null || { echo "Не найдена команда: $tool" >&2; exit 1; }
done

version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)"
[[ -n "$version" ]] || { echo 'Не удалось прочитать версию из Cargo.toml' >&2; exit 1; }
architecture="$(dpkg --print-architecture)"
package_version="${version}-1"
output_dir="$project_dir/dist/linux"
mkdir -p "$output_dir"

cargo build --release --locked

work_dir="$(mktemp -d "${TMPDIR:-/tmp}/pg-debugger-deb.XXXXXXXX")"
cleanup() {
    case "$work_dir" in
        "${TMPDIR:-/tmp}"/pg-debugger-deb.*) rm -rf -- "$work_dir" ;;
        *) echo "Неожиданный временный путь: $work_dir" >&2 ;;
    esac
}
trap cleanup EXIT

package_root="$work_dir/package"
install -Dm755 target/release/pg-debugger "$package_root/usr/bin/pg-debugger"
install -Dm644 debug_smoke.sql "$package_root/usr/share/pg-debugger/examples/debug_smoke.sql"
install -Dm644 debug_failure.sql "$package_root/usr/share/pg-debugger/examples/debug_failure.sql"
mkdir -p "$package_root/DEBIAN"

cat > "$package_root/DEBIAN/control" <<EOF
Package: pg-debugger
Version: $package_version
Section: devel
Priority: optional
Architecture: $architecture
Maintainer: ${PGDEBUG_DEB_MAINTAINER:-Local project build <noreply@localhost>}
Depends: libc6 (>= 2.34), libgcc-s1 (>= 3.0)
Description: Console debugger prototype for PostgreSQL PL/pgSQL
 Rust client for pldbgapi with an interactive debugger and an optional
 isolated demonstration cluster. A compatible PostgreSQL debugger plugin
 must be installed on the server separately.
EOF

package_file="$output_dir/pg-debugger_${package_version}_${architecture}.deb"
dpkg-deb --root-owner-group --build "$package_root" "$package_file"

tar_root="$work_dir/pg-debugger-${version}-linux-${architecture}"
mkdir -p "$tar_root"
install -m755 target/release/pg-debugger "$tar_root/pg-debugger"
install -m644 debug_smoke.sql "$tar_root/debug_smoke.sql"
install -m644 debug_failure.sql "$tar_root/debug_failure.sql"
archive_file="$output_dir/pg-debugger_${version}_linux_${architecture}.tar.gz"
tar -C "$work_dir" -czf "$archive_file" "$(basename "$tar_root")"

(
    cd "$output_dir"
    sha256sum "$(basename "$package_file")" "$(basename "$archive_file")" > SHA256SUMS
)

printf 'Готово:\n%s\n%s\n%s\n' "$package_file" "$archive_file" "$output_dir/SHA256SUMS"
