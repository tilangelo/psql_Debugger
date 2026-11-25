"""PostgreSQL исполняет код конструктора: SELECT с JOIN и вложенные операции."""
import copy,json,os,re,subprocess,time,urllib.request,urllib.error
from pathlib import Path
base=os.environ['PGDEBUG_WEB_TEST_URL'];password=os.environ['PGDEBUG_GUI_TEST_PASSWORD']
page=urllib.request.urlopen(base).read().decode();token=re.search(r'name="pg-debugger-token" content="([a-f0-9]+)"',page).group(1)
headers={'Content-Type':'application/json','x-pg-debugger-token':token}
credentials={'host':'127.0.0.1','port':os.environ['PGDEBUG_GUI_TEST_PORT'],'database':'gui_plugin_test','user':'debugger_test','password':password}
def post(path,body,status=200):
    try:
        with urllib.request.urlopen(urllib.request.Request(base+path,data=json.dumps(body).encode(),headers=headers),timeout=15) as r:code,text=r.status,r.read().decode()
    except urllib.error.HTTPError as e:code,text=e.code,e.read().decode()
    assert code==status,(path,code,status,text)
    assert password not in text
    return json.loads(text)
def sql(command,error=False):
    result=subprocess.run([os.environ.get('PGDEBUG_TEST_PSQL','/usr/lib/postgresql/17/bin/psql'),'-h','127.0.0.1','-p',credentials['port'],'-U','debugger_test','-d',credentials['database'],'-XAt','-v','ON_ERROR_STOP=1','-v','VERBOSITY=verbose','-c',command],env={**os.environ,'PGPASSWORD':password},capture_output=True,text=True)
    assert (result.returncode!=0)==error,(result.stdout,result.stderr)
    return result.stderr if error else result.stdout.strip()
identity=post('/api/connect',credentials)['connection_id']
sql('''CREATE SCHEMA join_checks;
CREATE TABLE join_checks.region(tenant integer PRIMARY KEY, label text);
CREATE TABLE join_checks.parent(tenant integer, key integer, label text, region_id integer REFERENCES join_checks.region(tenant), PRIMARY KEY(tenant,key));
CREATE TABLE join_checks.child(id integer PRIMARY KEY, tenant integer, parent_key integer, amount integer,
 FOREIGN KEY(tenant,parent_key) REFERENCES join_checks.parent(tenant,key));
INSERT INTO join_checks.region VALUES(1,'North');
INSERT INTO join_checks.parent VALUES(1,7,'A',1),(2,7,'B',NULL);
INSERT INTO join_checks.child VALUES(11,1,7,10),(12,2,7,20),(13,NULL,NULL,30);
CREATE TABLE join_checks.staff(id integer PRIMARY KEY, manager_id integer REFERENCES join_checks.staff(id), name text);
INSERT INTO join_checks.staff VALUES(1,NULL,'Boss'),(2,1,'Employee');
GRANT USAGE ON SCHEMA join_checks TO gui_reader;
GRANT SELECT ON join_checks.child TO gui_reader;
GRANT SELECT(tenant,key) ON join_checks.parent TO gui_reader;
''')
model={'name':'Тестовые заказы','tables':[{'schema':'join_checks','name':'child','label':'<img src=x onerror=alert(1)>','description':'DROP TABLE join_checks.child;','columns':[{'name':'id','label':'Идентификатор','description':'Подсказка','values':[{'value':'11','label':'Первый'}]}]}]}
def project(case):
    return {'schemaVersion':2,'model':copy.deepcopy(model),'kind':'function','schema':'public','name':'enrich_'+case['name'],'returnType':case['returnType'],'parameters':[{'name':'id','type':'integer'}],'variables':[{'name':'result_value','type':case['returnType'],'initial':''}],'blocks':[{'id':'select','type':'select','target':'result_value','query':case['query']},{'id':'return','type':'return','expression':'result_value'}]}
def payload(p):return {'connection_id':identity,'database':credentials['database'],'project':p}
def install(p):
    preview=post('/api/prepare',payload(p))
    assert 'DROP TABLE' not in preview['after'] and '<img' not in preview['after']
    post('/api/apply',{**payload(p),'preview_id':preview['preview_id']})
    return preview
cases=json.loads((Path(__file__).parent/'join_queries.json').read_text())
for case in cases:
    p=project(case);install(p)
    call=f'SELECT public.{p["name"]}({case["argument"]})'
    if case.get('expectedError'):assert case['expectedError'] in sql(call,error=True)
    elif case['expected'] is None:assert sql(f'SELECT public.{p["name"]}({case["argument"]}) IS NULL')=='t'
    else:assert sql(call)==case['expected'],case

loop=project(cases[0])
loop.update(name='enrich_loops',returnType='integer',variables=[{'name':'result_value','type':'integer','initial':'0'}],blocks=[
    {'id':'for','type':'for','iterator':'i','lower':'1','upper':'id','body':[
        {'id':'sum','type':'assign','target':'result_value','expression':'result_value + i'}]},
    {'id':'while','type':'while','condition':'result_value < 8','body':[
        {'id':'increment','type':'assign','target':'result_value','expression':'result_value + 1'}]},
    {'id':'return','type':'return','expression':'result_value'},
])
install(loop)
assert sql('SELECT enrich_loops(0), enrich_loops(3), enrich_loops(5)')=='8|8|15'

# Изменение подписей не меняет DDL, но старый предпросмотр не применяется к другому проекту.
p=project(cases[0]);preview=post('/api/prepare',payload(p));changed=copy.deepcopy(p);changed['model']['name']='Другая модель'
post('/api/apply',{**payload(changed),'preview_id':preview['preview_id']},409)
assert post('/api/prepare',payload(changed))['after']==preview['after']
bad=copy.deepcopy(p);bad['model']['tables'][0]['columns'][0]['values']*=2
post('/api/prepare',payload(bad),400)
bad=copy.deepcopy(p);bad['model']['sql']='DROP TABLE data';post('/api/prepare',payload(bad),422)
assert sql('SELECT count(*) FROM join_checks.child')=='3'

# Доступные ключи не дают доступа к неразрешённому полю результата связанной таблицы.
reader=post('/api/connect',{**credentials,'user':'gui_reader'})['connection_id']
body={'connection_id':reader,'database':credentials['database'],'schema':'join_checks','relation_oid':None}
data=post('/api/metadata',body)['metadata'];parent=next(r for r in data['relations'] if r['name']=='parent')
details=post('/api/metadata',{**body,'relation_oid':parent['oid']})['metadata']['selected']
assert [c['name'] for c in details['columns']]==['tenant','key']
identity=post('/api/connect',credentials)['connection_id']

# Функция и процедура с моделью v2 проходят настоящий сеанс, карту блоков и откат.
def browse(name):
    body={'connection_id':identity,'database':credentials['database'],'schema':'public'}
    routine=next(r for r in post('/api/catalog',body)['routines'] if r['name']==name)
    return post('/api/catalog',{**body,'routine_oid':routine['oid']})['selected']
def wait(state,phase):
    deadline=time.monotonic()+15
    while state['phase']!=phase:
        assert time.monotonic()<deadline and (state['phase']!='finished' or phase=='finished'),state
        time.sleep(.1);state=post('/api/debug/poll',{'session_id':state['session_id']})
    return state
def command(state,action):return post('/api/debug/command',{'session_id':state['session_id'],'revision':state['revision'],'action':{'command':action}})
for kind in ['function','procedure']:
    p=project(cases[0]);p['kind']=kind;p['name']='enrich_debug_'+kind;install(p);details=browse(p['name'])
    state=post('/api/debug/start',{'connection_id':identity,'database':credentials['database'],'schema':'public','routine_oid':details['oid'],'expected_definition':details['definition'],'values':['11'],'breakpoints':[],'project':p})
    state=wait(state,'paused');assert state['graph_mapping'] and state['position']['block_id']=='select'
    assert state['project']['model']['name']==model['name']
    state=wait(command(state,'next'),'paused');assert next(v['value'] for v in state['variables'] if v['name']=='result_value')=='A'
    state=wait(command(state,'continue'),'finished');assert state['outcome']['call']=='succeeded' and state['outcome']['rollback']=='confirmed'
post('/api/disconnect',{})
print('HTTP enrichment: exact JS JOIN SQL, chained/composite/null/self links, FOR/WHILE, scalar 21000, model, stale preview, privileges, functions/procedures and rollback passed.')
