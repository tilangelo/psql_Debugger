"""Фактические проекты шаблонов, настройки SELECT v3 и откат изменяющей процедуры."""
import copy,json,os,re,subprocess,time,urllib.request,urllib.error
from pathlib import Path
base=os.environ['PGDEBUG_WEB_TEST_URL'];password=os.environ['PGDEBUG_GUI_TEST_PASSWORD']
page=urllib.request.urlopen(base).read().decode();token=re.search(r'name="pg-debugger-token" content="([a-f0-9]+)"',page).group(1)
headers={'Content-Type':'application/json','x-pg-debugger-token':token}
credentials={'host':'127.0.0.1','port':os.environ['PGDEBUG_GUI_TEST_PORT'],'database':'gui_plugin_test','user':'debugger_test','password':password}
def post(path,body=None,status=200):
    try:
        with urllib.request.urlopen(urllib.request.Request(base+path,data=json.dumps(body or {}).encode(),headers=headers),timeout=15) as response:code,text=response.status,response.read().decode()
    except urllib.error.HTTPError as e:code,text=e.code,e.read().decode()
    assert code==status,(path,code,status,text)
    assert password not in text
    return json.loads(text)
def sql(command,error=False):
    result=subprocess.run([os.environ.get('PGDEBUG_TEST_PSQL','/usr/lib/postgresql/17/bin/psql'),'-h','127.0.0.1','-p',credentials['port'],'-U','debugger_test','-d',credentials['database'],'-XAt','-v','ON_ERROR_STOP=1','-v','VERBOSITY=verbose','-c',command],env={**os.environ,'PGPASSWORD':password},capture_output=True,text=True)
    assert (result.returncode!=0)==error,(result.stdout,result.stderr)
    return result.stderr if error else result.stdout.strip()
expected_version=re.search(r'^version = "([^"]+)"',(Path(__file__).resolve().parents[1]/'Cargo.toml').read_text(),re.M).group(1)
assert post('/api/info')['version']==expected_version
catalog=post('/api/connect',credentials);identity=catalog['connection_id'];assert catalog['server_version'] and catalog['current_user']=='debugger_test'
sql("CREATE SCHEMA template_checks; CREATE TABLE template_checks.orders(id integer PRIMARY KEY,amount integer,status text); INSERT INTO template_checks.orders VALUES(1,10,'new'),(2,10,'new'),(3,30,'new')")
projects=json.loads((Path(__file__).parent/'template_projects.json').read_text())
def install(project):
    payload={'connection_id':identity,'database':credentials['database'],'project':project}
    preview=post('/api/prepare',payload);post('/api/apply',{**payload,'preview_id':preview['preview_id']})
for project in projects:install(project)
assert sql('SELECT public.template_lookup(1)')=='new'
assert sql('SELECT public.template_lookup(999) IS NULL')=='t'
assert sql('SELECT public.template_sum()')=='50'
sql("CALL public.template_status(1,'paid')");assert sql('SELECT status FROM template_checks.orders WHERE id=1')=='paid'
sql("UPDATE template_checks.orders SET status='new'")
assert 'P0002' in sql("CALL public.template_status(999,'paid')",True)
multiple=copy.deepcopy(projects[2]);multiple['name']='template_multiple';multiple['blocks'][0]['sql']=multiple['blocks'][0]['sql'].replace('WHERE "id"','WHERE "amount"');install(multiple)
assert '21000' in sql("CALL public.template_multiple(10,'paid')",True)
assert sql("SELECT count(*) FROM template_checks.orders WHERE status='paid'")=='0'
bad=copy.deepcopy(projects[0]);bad['schemaVersion']=2
post('/api/prepare',{'connection_id':identity,'database':credentials['database'],'project':bad},400)
bad=copy.deepcopy(projects[0]);bad['blocks'][0]['assistant']['password']='do-not-echo'
post('/api/prepare',{'connection_id':identity,'database':credentials['database'],'project':bad},422)
def browse(name):
    payload={'connection_id':identity,'database':credentials['database'],'schema':'public'}
    routine=next(r for r in post('/api/catalog',payload)['routines'] if r['name']==name)
    return post('/api/catalog',{**payload,'routine_oid':routine['oid']})['selected']
def wait(state,phase):
    deadline=time.monotonic()+15
    while state['phase']!=phase:
        assert time.monotonic()<deadline and (state['phase']!='finished' or phase=='finished'),state
        time.sleep(.1);state=post('/api/debug/poll',{'session_id':state['session_id']})
    return state
def command(state,command):return post('/api/debug/command',{'session_id':state['session_id'],'revision':state['revision'],'action':{'command':command}})
def start(project,values):
    selected=browse(project['name'])
    return wait(post('/api/debug/start',{'connection_id':identity,'database':credentials['database'],'schema':'public','routine_oid':selected['oid'],'expected_definition':selected['definition'],'values':values,'breakpoints':[],'project':project}),'paused')
state=start(projects[0],['1']);assert state['project']['blocks'][0]['assistant']['table']=='orders'
assert state['server_version'] and state['current_user']=='debugger_test'
state=wait(command(state,'continue'),'finished');assert state['outcome']['call']=='succeeded'
state=start(projects[2],['1','paid']);state=wait(command(state,'next'),'paused');state=wait(command(state,'next'),'paused')
assert next(v['value'] for v in state['variables'] if v['name']=='changed_rows')=='1'
state=wait(command(state,'continue'),'finished');assert state['outcome']['call']=='succeeded' and state['outcome']['rollback']=='confirmed'
assert sql("SELECT count(*) FROM template_checks.orders WHERE status='paid'")=='0'
state=start(projects[2],['999','paid']);state=wait(command(state,'continue'),'finished');assert state['outcome']['sqlstate']=='P0002' and state['outcome']['rollback']=='confirmed'
post('/api/disconnect')
print('HTTP checkpoint: v3 settings, real lookup/sum/status templates, affected-row guards, rollback, SQLSTATE and diagnostic environment passed.')
