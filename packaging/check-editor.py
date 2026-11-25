"""Проверка готового интерфейса без Node.js и обращения в сеть."""
from pathlib import Path
import hashlib,json
root=Path(__file__).resolve().parents[1]/'web/prototype'
expected={'editor-src/code-editor.js','build-editor.mjs','package.json','package-lock.json','vendor/code-editor.js','vendor/THIRD_PARTY_LICENSES.txt'}
hashes=json.loads((root/'vendor/build.json').read_text())
assert set(hashes)==expected,'Unexpected editor manifest; rebuild with npm run build:editor'
for name in sorted(expected):
    assert hashlib.sha256((root/name).read_bytes()).hexdigest()==hashes[name],f'Editor artifact is stale: {name}. Run npm ci --ignore-scripts and npm run build:editor.'
print('Offline editor: source, lockfile, bundle and third-party notices match.')
