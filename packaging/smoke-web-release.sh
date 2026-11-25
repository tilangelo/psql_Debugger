#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' web/server/Cargo.toml | head -n 1)"
name="pg-debugger-web-$version-debian13-$(dpkg --print-architecture)"
archive="$name.tar.gz"
(cd dist/web; sha256sum -c "$archive.sha256")
work="$(mktemp -d)"
tar -xzf "dist/web/$archive" -C "$work"
binary="$work/$name/pg-debugger-web"
test -s "$work/$name/README.md"
test -s "$work/$name/THIRD_PARTY_LICENSES.txt"
if ldd "$binary" | grep 'not found'; then exit 1; fi
"$binary" --help
"$binary" --port 18768 --no-browser >"$work/log" 2>&1 &
pid=$!
cleanup(){ kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; rm -rf -- "$work"; }
trap cleanup EXIT
python3 - <<'PY'
import urllib.request,re,json,time
url='http://127.0.0.1:18768'
for _ in range(40):
    try:
        page=urllib.request.urlopen(url,timeout=2).read().decode();break
    except OSError: time.sleep(.1)
else: raise AssertionError('Archive executable did not start')
token=re.search(r'name="pg-debugger-token" content="([a-f0-9]+)"',page).group(1)
request=urllib.request.Request(url+'/api/disconnect',data=b'{}',headers={'Content-Type':'application/json','x-pg-debugger-token':token})
assert json.load(urllib.request.urlopen(request))['disconnected']
for asset in ['help.js','data-types.js','declarations.js','debug-presentation.js','working-draft.js','history.js','history-ui.js','recovery-store.js','recovery.js','body-target.js','vendor/code-editor.js','vendor/THIRD_PARTY_LICENSES.txt','sql-hints.js','sql-field.js','sql-completions.js','metadata-reader.js','metadata-client.js','readiness.js','body-editor.js','app.js','model.js','catalog.js','debug.js','nested-point-plan.js','trigger-call.js','work-presets.js','metadata.js','select-builder.js','database-client.js','model-hints.js','hints-editor.js','select-settings.js','templates.js','template-panel.js','workspace.js','workspace.css','diagnostics.js','diagnostic-report.js','style.css']:
    assert len(urllib.request.urlopen(url+'/'+asset).read())>100
print('Extracted release starts and serves embedded UI and protected API.')
PY
