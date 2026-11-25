"""Редактор распространяется внутри сервера, без CDN и Node.js при запуске."""
import os,re,urllib.request
base=os.environ['PGDEBUG_WEB_TEST_URL']
with urllib.request.urlopen(base) as response:
    page=response.read().decode();policy=response.headers['Content-Security-Policy']
assert "script-src 'self'" in policy and 'unsafe-eval' not in policy
assert not re.search(r"script-src[^;]*unsafe-inline",policy)
assert "style-src 'self' 'nonce-" in policy and "style-src-attr 'unsafe-inline'" in policy
assert 'id="undo"' in page and 'id="recovery-notices"' in page
for name in ['help.js','data-types.js','declarations.js','debug-presentation.js','working-draft.js','history.js','history-ui.js','recovery-store.js','recovery.js','body-target.js','metadata-reader.js','metadata-client.js','sql-completions.js','sql-hints.js','sql-field.js','vendor/code-editor.js']:
    with urllib.request.urlopen(base+'/'+name) as response:
        text=response.read().decode();assert text and 'javascript' in response.headers['Content-Type']
        if name=='vendor/code-editor.js':
            assert len(text)>100000 and 'createCodeEditor' in text
            assert not re.search(r'(^|;)\s*import\s',text),'Bundle still contains external imports'
notice=urllib.request.urlopen(base+'/vendor/THIRD_PARTY_LICENSES.txt').read().decode()
assert '@codemirror/view@' in notice and 'Permission is hereby granted' in notice
print('HTTP editor assets: bundled editor, shared hints, third-party notices, nonce styles and unchanged script restrictions passed.')
