"""Проверки настоящего HTTP-сервера на временных базах общего стенда."""
import json, os, re, urllib.request, urllib.error, time

base=os.environ.get('PGDEBUG_WEB_TEST_URL','http://127.0.0.1:18766')
for attempt in range(80):
    try:
        with urllib.request.urlopen(base,timeout=2) as response:
            html=response.read().decode()
        break
    except OSError:
        time.sleep(.1)
else:
    raise AssertionError('Web server did not start')
token=re.search(r'name="pg-debugger-token" content="([a-f0-9]+)"',html).group(1)
password=os.environ['PGDEBUG_GUI_TEST_PASSWORD']
headers={'Content-Type':'application/json','x-pg-debugger-token':token}
def post(path,data,expected=200,extra=None):
    request=urllib.request.Request(base+path,data=json.dumps(data).encode(),headers={**headers,**(extra or {})},method='POST')
    try:
        with urllib.request.urlopen(request,timeout=12) as response:
            code,text=response.status,response.read().decode()
    except urllib.error.HTTPError as error:
        code,text=error.code,error.read().decode()
    assert code==expected,(path,code,expected)
    assert password not in text,'Credential leaked in response'
    return json.loads(text)
credentials={'host':'127.0.0.1','port':os.environ['PGDEBUG_GUI_TEST_PORT'],'database':'gui_catalog_test','user':'debugger_test','password':password}
catalog=post('/api/connect',credentials)
assert catalog['endpoint']=={'host':'127.0.0.1','port':int(credentials['port'])}
identity=catalog['connection_id']
assert 'gui_plugin_test' in catalog['databases']
assert 'gui_checks' in catalog['schemas']
def browse(**selection):
    return post('/api/catalog',{'connection_id':identity,'database':'gui_catalog_test','schema':'gui_checks',**selection})
catalog=browse()
assert len([r for r in catalog['routines'] if r['name']=='calculate'])==2
assert not any(r['name']=='sql_function' for r in catalog['routines'])
routine=next(r for r in catalog['routines'] if r['name']=='calculate' and 'integer' in r['signature'])
details=browse(routine_oid=routine['oid'])['selected']
assert 'input_value + 1' in details['source']
assert 'CREATE OR REPLACE FUNCTION' in details['definition']
assert details['arguments'][0]['type']=='integer'
procedure=next(r for r in catalog['routines'] if r['name']=='process')
details=browse(routine_oid=procedure['oid'])['selected']
assert 'PROCEDURE' in details['definition'] and len(details['arguments'])==3
assert 'pldbgapi' in catalog['debugger']
catalog=browse(database='gui_plugin_test',schema='public')
assert any(r['name']=='debug_smoke' for r in catalog['routines'])
post('/api/catalog',{'connection_id':identity+999,'database':'gui_catalog_test'},409)
post('/api/connect',{**credentials,'password':'wrong-password'},400)
browse() # Ошибка нового подключения не уничтожает прежнее.
post('/api/disconnect',{},403,{'Origin':'http://foreign.example'})
post('/api/disconnect',{})
post('/api/catalog',{'connection_id':identity,'database':'gui_catalog_test'},409)
print('HTTP catalog: connection, databases, schemas, overloads, source, procedure, missing extension, database switch, stale IDs, redaction and disconnect passed.')

setup_database = 'gui_setup_test'
reader = post('/api/connect', {**credentials, 'database': setup_database, 'user': 'gui_reader'})
assert reader['debugger_setup']['package_available'] and not reader['debugger_setup']['extension_created']
assert not reader['debugger_setup']['superuser']
attempt = {'connection_id': reader['connection_id'], 'database': setup_database}
post('/api/debug/setup', attempt, 403, {'Origin': 'http://foreign.example'})
denied = post('/api/debug/setup', attempt, 400)
assert 'суперпользователя' in denied['error']
admin = post('/api/connect', {**credentials, 'database': setup_database})
assert not admin['debugger_setup']['extension_created']
post('/api/debug/setup', attempt, 409)
attempt['connection_id'] = admin['connection_id']
enabled = post('/api/debug/setup', attempt)
assert enabled['debugger_code'] == 'configured'
assert 'pgdebugger_tools' in enabled['schemas']
assert post('/api/debug/setup', attempt)['debugger_setup']['extension_created']
assert not browse(database='gui_catalog_test', connection_id=admin['connection_id'])['debugger_setup']['extension_created']
post('/api/disconnect', {})
print('HTTP debugger setup: role, origin, stale connection, target database, private schema and re-entry passed.')
