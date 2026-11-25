"""Структура и подсказки на настоящей временной базе, включая ограничения прав."""
import copy, json, os, re, subprocess, time, urllib.request, urllib.error

base = os.environ['PGDEBUG_WEB_TEST_URL']
password = os.environ['PGDEBUG_GUI_TEST_PASSWORD']
page = urllib.request.urlopen(base).read().decode()
token = re.search(r'name="pg-debugger-token" content="([a-f0-9]+)"', page).group(1)
headers = {'Content-Type':'application/json', 'x-pg-debugger-token':token}
credentials = {'host':'127.0.0.1', 'port':os.environ['PGDEBUG_GUI_TEST_PORT'], 'database':'gui_shop_test', 'user':'debugger_test', 'password':password}

def post(path, body, expected=200):
    try:
        with urllib.request.urlopen(urllib.request.Request(base+path, data=json.dumps(body).encode(), headers=headers), timeout=15) as response:
            code, text = response.status, response.read().decode()
    except urllib.error.HTTPError as error:
        code, text = error.code, error.read().decode()
    assert code == expected, (path, code, expected, text)
    assert password not in text, 'Credential leaked'
    return json.loads(text)

def sql(command, expected=0):
    result = subprocess.run([os.environ.get('PGDEBUG_TEST_PSQL','/usr/lib/postgresql/17/bin/psql'), '-h','127.0.0.1','-p',credentials['port'],'-U','debugger_test','-d',credentials['database'],'-XAt','-v','ON_ERROR_STOP=1','-v','VERBOSITY=verbose','-c',command], env={**os.environ,'PGPASSWORD':password}, capture_output=True,text=True)
    assert (result.returncode == 0) == (expected == 0), (result.stdout, result.stderr)
    return result.stdout.strip() if expected == 0 else result.stderr

identity = post('/api/connect', credentials)['connection_id']
def metadata(schema='sales', oid=None, expected=200, **extra):
    response = post('/api/metadata', {'connection_id':identity,'database':credentials['database'],'schema':schema,'relation_oid':oid,**extra},expected)
    return response.get('metadata',response)

catalog = metadata()
assert 'sales' in catalog['schemas'] and catalog['selected'] is None
orders = next(r for r in catalog['relations'] if r['name']=='orders')
details = metadata(oid=orders['oid'])['selected']
assert details['primary_keys'][0]['columns'] == ['id']
assert details['foreign_keys'][0]['columns'] == ['customer_id']
assert details['foreign_keys'][0]['referenced_table'] == 'customers'
items = next(r for r in catalog['relations'] if r['name']=='order_items')
assert metadata(oid=items['oid'])['selected']['primary_keys'][0]['columns'] == ['order_id','product']
metadata('billing',orders['oid'],400) # OID другой схемы не принимается.
metadata("sales'; DROP SCHEMA sales CASCADE; --",expected=400)
metadata(connection_id=identity+1000,expected=409)

sql('''CREATE SCHEMA pguser;
CREATE SCHEMA "Meta test";
CREATE TABLE "Meta test".parent(tenant integer, id integer, PRIMARY KEY(tenant,id));
CREATE TABLE "Meta test".child(id integer PRIMARY KEY, tenant integer, parent_id integer, shown text, hidden text, removed integer,
 FOREIGN KEY(tenant,parent_id) REFERENCES "Meta test".parent(tenant,id));
ALTER TABLE "Meta test".child DROP COLUMN removed;
CREATE TABLE "Meta test".private_data(secret text);
CREATE SCHEMA metadata_secret;
REVOKE ALL ON SCHEMA metadata_secret FROM PUBLIC;
CREATE TABLE metadata_secret.hidden_table(id integer);
COMMENT ON TABLE "Meta test".child IS '<img src=x onerror=alert(1)>';
COMMENT ON COLUMN "Meta test".child.shown IS 'Visible comment';
CREATE VIEW "Meta test".child_view AS SELECT shown FROM "Meta test".child;
CREATE MATERIALIZED VIEW "Meta test".child_materialized AS SELECT shown FROM "Meta test".child;
CREATE TABLE "Meta test".partitioned(id integer) PARTITION BY RANGE(id);
CREATE TABLE "Meta test".partition_one PARTITION OF "Meta test".partitioned FOR VALUES FROM(0) TO(10);
GRANT USAGE ON SCHEMA "Meta test" TO gui_reader;
GRANT SELECT(tenant,parent_id,shown) ON "Meta test".child TO gui_reader;
GRANT SELECT(tenant,id) ON "Meta test".parent TO gui_reader;
CREATE TABLE "Meta test"."Values \""table"(id integer, amount integer);
INSERT INTO "Meta test"."Values \""table" VALUES (7,10),(7,20),(8,30);
''')
data = metadata('Meta test')
assert 'pguser' in data['schemas']
assert {'r','v','m','p'}.issubset({r['kind'] for r in data['relations']})
child = next(r for r in data['relations'] if r['name']=='child')
assert child['comment'].startswith('<img')
details = metadata('Meta test',child['oid'])['selected']
assert not any(c['name']=='removed' for c in details['columns'])
fk = details['foreign_keys'][0]
assert fk['columns']==['tenant','parent_id'] and fk['referenced_columns']==['tenant','id']
identity = post('/api/connect',{**credentials,'user':'gui_reader'})['connection_id']
data = metadata('Meta test')
assert not any(r['name']=='private_data' for r in data['relations'])
assert 'metadata_secret' not in data['schemas']
reader_child = next(r for r in data['relations'] if r['name']=='child')
assert not reader_child['can_select_all']
details = metadata('Meta test',child['oid'])['selected']
assert [c['name'] for c in details['columns']] == ['tenant','parent_id','shown']
assert details['primary_keys'] == [] and len(details['foreign_keys']) == 1
sql('REVOKE SELECT(id) ON "Meta test".parent FROM gui_reader')
assert metadata('Meta test',child['oid'])['selected']['foreign_keys'] == []
sql('REVOKE SELECT(tenant,parent_id,shown) ON "Meta test".child FROM gui_reader')
metadata('Meta test',child['oid'],400)

identity = post('/api/connect',credentials)['connection_id']
project = {'schemaVersion':1,'kind':'function','schema':'public','name':'meta_lookup','returnType':'integer','parameters':[{'name':'id','type':'integer'}],'variables':[{'name':'result_value','type':'integer','initial':''}],'blocks':[{'id':'select','type':'select','target':'result_value','query':'SELECT (SELECT "__pgdbg_row"."amount" FROM "Meta test"."Values ""table" AS "__pgdbg_row" WHERE "__pgdbg_row"."id" = $1)'},{'id':'return','type':'return','expression':'result_value'}]}
def install(p):
    body = {'connection_id':identity,'database':credentials['database'],'project':p}
    preview = post('/api/prepare',body)
    post('/api/apply',{**body,'preview_id':preview['preview_id']})
install(project)
assert sql('SELECT public.meta_lookup(8)')=='30'
assert sql('SELECT public.meta_lookup(999) IS NULL')=='t'
assert '21000' in sql('SELECT public.meta_lookup(7)',expected=1)
for mode, expected in [('count','2'),('sum','30'),('min','10'),('max','20')]:
    aggregate = copy.deepcopy(project);aggregate['name']='meta_'+mode
    expression = '*' if mode=='count' else '"__pgdbg_row"."amount"'
    aggregate['blocks'][0]['query'] = f'SELECT pg_catalog.{mode}({expression}) FROM "Meta test"."Values ""table" AS "__pgdbg_row" WHERE "__pgdbg_row"."id" = $1'
    install(aggregate)
    assert sql(f'SELECT public.meta_{mode}(7)')==expected

# Несколько строк показываются в отладчике именно как ошибка SQL с подтверждённым откатом.
catalog = post('/api/catalog',{'connection_id':identity,'database':credentials['database'],'schema':'public'})
routine = next(r for r in catalog['routines'] if r['name']=='meta_lookup')
details = post('/api/catalog',{'connection_id':identity,'database':credentials['database'],'schema':'public','routine_oid':routine['oid']})['selected']
state = post('/api/debug/start',{'connection_id':identity,'database':credentials['database'],'schema':'public','routine_oid':details['oid'],'expected_definition':details['definition'],'values':['7'],'breakpoints':[],'project':project})
deadline = time.monotonic()+15
while state['phase']!='paused':
    assert time.monotonic()<deadline and state['phase']!='finished',state
    time.sleep(.1);state=post('/api/debug/poll',{'session_id':state['session_id']})
metadata('sales',expected=409) # Структура использует тот же слот базы, что и отладка.
post('/api/debug/command',{'session_id':state['session_id'],'revision':state['revision'],'action':{'command':'continue'}})
while state['phase']!='finished':
    assert time.monotonic()<deadline,state
    time.sleep(.1);state=post('/api/debug/poll',{'session_id':state['session_id']})
assert state['outcome']['call']=='failed' and state['outcome']['rollback']=='confirmed'
assert '21000' in state['outcome']['text']
post('/api/disconnect',{})
metadata(expected=409)
print('HTTP metadata: shop keys, composite FK order, dropped columns, views, partitioned tables, column/schema privileges, revocation, quoted names, scalar NULL/21000, aggregates, debug rollback and occupied slot passed.')
