"""Новая тестовая база, права, защита старых баз и настоящий вызов из неё."""
import json,os,re,subprocess,time,urllib.request,urllib.error
base=os.environ['PGDEBUG_WEB_TEST_URL'];password=os.environ['PGDEBUG_GUI_TEST_PASSWORD']
page=urllib.request.urlopen(base).read().decode()
assert 'start-screen' in page and 'Открыть тестовый пример' in page
token=re.search(r'name="pg-debugger-token" content="([a-f0-9]+)"',page).group(1)
headers={'Content-Type':'application/json','x-pg-debugger-token':token}
credentials={'host':'127.0.0.1','port':os.environ['PGDEBUG_GUI_TEST_PORT'],'database':'gui_plugin_test','user':'debugger_test','password':password}
def post(path,body=None,status=200):
    try:
        with urllib.request.urlopen(urllib.request.Request(base+path,data=json.dumps(body or {}).encode(),headers=headers),timeout=45) as response:code,text=response.status,response.read().decode()
    except urllib.error.HTTPError as err:code,text=err.code,err.read().decode()
    assert code==status,(path,code,status,text)
    assert password not in text
    return json.loads(text)
def sql(command,database='gui_plugin_test'):
    return subprocess.check_output([os.environ.get('PGDEBUG_TEST_PSQL','/usr/lib/postgresql/17/bin/psql'),'-h','127.0.0.1','-p',credentials['port'],'-U','debugger_test','-d',database,'-XAt','-v','ON_ERROR_STOP=1','-c',command],env={**os.environ,'PGPASSWORD':password},text=True).strip()
def count():return sql("SELECT count(*) FROM pg_database WHERE datname LIKE 'pgdebug_demo_%'")
catalog=post('/api/connect',{**credentials,'database':'gui_catalog_test'})
assert catalog['debugger_code']=='missing_extension'
catalog=post('/api/connect',credentials);assert catalog['debugger_code']=='configured'
identity=catalog['connection_id'];before=count()
post('/api/demo',{'connection_id':identity,'database':credentials['database'],'confirmed':False},400)
post('/api/demo',{'connection_id':identity+99,'database':credentials['database'],'confirmed':True},409)
assert count()==before
reader=post('/api/connect',{**credentials,'user':'gui_reader'})
denied=post('/api/demo',{'connection_id':reader['connection_id'],'database':credentials['database'],'confirmed':True},400)
assert 'администратора' in denied['error'] and count()==before
catalog=post('/api/connect',credentials);identity=catalog['connection_id']
created=post('/api/demo',{'connection_id':identity,'database':credentials['database'],'confirmed':True})
name=created['database'];assert re.fullmatch(r'pgdebug_demo_[a-f0-9]{16}',name)
assert created['connection_id']!=identity and created['debugger_code']=='configured'
assert created['selected'] and created['selected']['arguments'][0]['type']=='integer'
assert 'sales' in created['schemas'] and created['demo']['expected']=='8'
assert sql('SELECT public.debug_smoke(3)',name)=='8'
assert sql('SELECT sales.order_total(1001)',name)=='5990'
assert sql("SELECT count(*) FROM pg_namespace WHERE nspname='sales'")=='0','Original database was populated'
post('/api/demo',{'connection_id':identity,'database':credentials['database'],'confirmed':True},409)
selected=created['selected']
state=post('/api/debug/start',{'connection_id':created['connection_id'],'database':name,'schema':'public','routine_oid':selected['oid'],'expected_definition':selected['definition'],'values':['3'],'breakpoints':[],'project':None})
def wait(state,phase):
    until=time.monotonic()+15
    while state['phase']!=phase:
        assert time.monotonic()<until,(phase,state)
        time.sleep(.1);state=post('/api/debug/poll',{'session_id':state['session_id']})
    return state
state=wait(state,'paused');assert state['position']['line']==5
state=post('/api/debug/command',{'session_id':state['session_id'],'revision':state['revision'],'action':{'command':'continue'}})
state=wait(state,'finished');assert state['outcome']['call']=='succeeded' and state['outcome']['rollback']=='confirmed'
assert '8' in state['outcome']['text']
post('/api/disconnect')
# Удаляется только база, созданная этим тестом на его собственном временном сервере.
sql('DROP DATABASE "'+name+'"')
assert count()==before
print('HTTP onboarding: readiness, explicit creation, denied role, stale replay, isolated demo, real result 8 and rollback passed.')
