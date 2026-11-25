"""Штатный запуск и остановка временного PostgreSQL через веб-бинарник."""
import json
import os
from pathlib import Path
import pwd
import re
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request


def wait_for(check, description, timeout=18):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(.1)
    raise AssertionError(description)


binary = Path(sys.argv[1]).resolve(strict=True)
account = pwd.getpwnam('postgres') if os.geteuid() == 0 else None
launch = dict(user=account.pw_uid, group=account.pw_gid, extra_groups=[]) if account else {}
with tempfile.TemporaryDirectory(prefix='pgdebug-web-demo-test-') as directory:
    root = Path(directory)
    if account:
        os.chown(root, account.pw_uid, account.pw_gid)
    log = root / 'web.log'
    with log.open('wb') as output:
        process = subprocess.Popen(
            [str(binary), '--port', '0', '--no-browser', '--demo'], stdout=output,
            stderr=subprocess.STDOUT, env={**os.environ, 'TMPDIR': directory}, **launch,
        )
    def read_log():
        return log.read_text(encoding='utf-8')
    try:
        def web_address():
            assert process.poll() is None, read_log()
            return re.search(r'http://127\.0\.0\.1:(\d+)', read_log())
        base = wait_for(web_address, 'Временный экземпляр не запустился', 35).group()
        page = urllib.request.urlopen(base, timeout=5).read().decode()
        assert 'name="pg-debugger-isolated-demo"' in page
        token = re.search(r'name="pg-debugger-token" content="([a-f0-9]+)"', page).group(1)

        def post(path, payload):
            request = urllib.request.Request(
                base + path, data=json.dumps(payload).encode(),
                headers={'content-type':'application/json', 'x-pg-debugger-token':token})
            with urllib.request.urlopen(request, timeout=7) as response:
                return json.load(response)

        demo = post('/api/demo/open-isolated', {})
        assert {'sales', 'billing', 'audit'}.issubset(demo['schemas']), demo['schemas']
        assert 'password' not in json.dumps(demo)
        reopened = post('/api/demo/open-isolated', {})
        assert demo['database'] == reopened['database'] and demo['endpoint'] == reopened['endpoint']
        selected = reopened['selected']
        payload = {'connection_id':reopened['connection_id'], 'database':reopened['database'],
                   'schema':'public', 'routine_oid':selected['oid'], 'expected_definition':selected['definition'],
                   'values':[reopened['demo']['argument']]}

        def start():
            session = post('/api/debug/start', payload)['session_id']
            def paused():
                state = post('/api/debug/poll', {'session_id':session})
                assert state['phase'] != 'finished', state
                return state if state['phase'] == 'paused' else None
            return wait_for(paused, 'Вызов не остановился')

        state = start()
        post('/api/debug/command', {'session_id':state['session_id'], 'revision':state['revision'],
                                    'action':{'command':'continue'}})
        def finished():
            current = post('/api/debug/poll', {'session_id':state['session_id']})
            return current if current['phase'] == 'finished' else None
        result = wait_for(finished, 'Вызов не завершился')['outcome']
        assert result['call'] == 'succeeded' and result['rollback'] == 'confirmed', result
        assert reopened['demo']['expected'] in result['text']
        start()  # На штатном выходе надо отменить даже вызов, оставшийся на паузе.
        pg_port = reopened['endpoint']['port']
    finally:
        if process.poll() is None:
            process.send_signal(signal.SIGINT)
            process.wait(timeout=25)
    assert process.returncode == 0, read_log()
    assert not list(root.glob('pg-debugger-demo-*'))
    try:
        with socket.create_connection(('127.0.0.1', pg_port), timeout=1):
            raise AssertionError('Временный PostgreSQL остался запущен.')
    except ConnectionRefusedError:
        pass
    print(f'Isolated web demo passed on PostgreSQL {demo["server_version"]}: reopen, real pause/result and teardown during a paused call.')
