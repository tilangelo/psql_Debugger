"""Новые типы через Project/DDL, настоящий вызов и pldbgapi, без float для точных значений."""
import copy,json,os,re,subprocess,time,urllib.request,urllib.error
from pathlib import Path
base=os.environ['PGDEBUG_WEB_TEST_URL'];password=os.environ['PGDEBUG_GUI_TEST_PASSWORD']
page=urllib.request.urlopen(base).read().decode();token=re.search(r'name="pg-debugger-token" content="([a-f0-9]+)"',page).group(1)
headers={'Content-Type':'application/json','x-pg-debugger-token':token}
credentials={'host':'127.0.0.1','port':os.environ['PGDEBUG_GUI_TEST_PORT'],'database':os.environ.get('PGDEBUG_SCALAR_TEST_DATABASE','gui_plugin_test'),'user':'debugger_test','password':password}
def post(path,body,status=200):
    try:
        with urllib.request.urlopen(urllib.request.Request(base+path,data=json.dumps(body).encode(),headers=headers),timeout=20) as response:code,text=response.status,response.read().decode()
    except urllib.error.HTTPError as error:code,text=error.code,error.read().decode()
    assert code==status,(path,code,status,text)
    assert password not in text
    return json.loads(text)
def sql(command):
    return subprocess.check_output([os.environ.get('PGDEBUG_TEST_PSQL','/usr/lib/postgresql/17/bin/psql'),'-h','127.0.0.1','-p',credentials['port'],'-U','debugger_test','-d',credentials['database'],'-XAt','-v','ON_ERROR_STOP=1','-c',command],env={**os.environ,'PGPASSWORD':password},text=True).strip()
identity=post('/api/connect',credentials)['connection_id']
sql('CREATE SCHEMA precise_types; CREATE TABLE precise_types.guard(amount numeric)')
def browse(name):
    payload={'connection_id':identity,'database':credentials['database'],'schema':'precise_types'}
    catalog=post('/api/catalog',payload);routine=next(r for r in catalog['routines'] if r['name']==name)
    return post('/api/catalog',{**payload,'routine_oid':routine['oid']})['selected']
def start(details,values,status=200,project=None,defaults=None):
    fields=[argument.get('fields') for argument in details['arguments']]
    return post('/api/debug/start',{'connection_id':identity,'database':credentials['database'],'schema':'precise_types','routine_oid':details['oid'],'expected_definition':details['definition'],'record_fields':fields,'values':values,'default_arguments':defaults or [],'breakpoints':[],'project':project},status)
def wait(state,phase):
    deadline=time.monotonic()+15
    while state['phase']!=phase:
        assert state['phase']!='finished' and time.monotonic()<deadline,state
        time.sleep(.1);state=post('/api/debug/poll',{'session_id':state['session_id']})
    return state
def finish_outcome(state):
    data=post('/api/debug/command',{'session_id':state['session_id'],'revision':state['revision'],'action':{'command':'continue'}})
    data=wait(data,'finished');assert data['outcome']['call']=='succeeded' and data['outcome']['rollback']=='confirmed',data
    return data['outcome']
def finish(state):
    return finish_outcome(state)['text']
sql("""
CREATE FUNCTION precise_types.table_preview(max_rows integer DEFAULT 105)
RETURNS TABLE ("item label" text, amount numeric) LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO precise_types.guard VALUES (1);
    RETURN QUERY SELECT CASE i WHEN 1 THEN NULL WHEN 2 THEN 'NULL' ELSE E'a\\tb\\nc' END,
        9007199254740993.123456789::numeric
    FROM generate_series(1, max_rows) i ORDER BY i;
END;
$$;
CREATE FUNCTION precise_types.scalar_set() RETURNS SETOF integer LANGUAGE plpgsql AS $$
BEGIN
    RETURN NEXT 11;
    RETURN NEXT NULL;
END;
$$;
CREATE TYPE precise_types.result_record AS (id integer, label text);
CREATE FUNCTION precise_types.composite_set() RETURNS SETOF precise_types.result_record LANGUAGE plpgsql AS $$
BEGIN
    RETURN NEXT (1,repeat('я',20000))::precise_types.result_record;
END;
$$;
""")
def table_result(name, values=(), defaults=None):
    return finish_outcome(wait(start(browse(name),list(values),defaults=defaults),'paused'))['result']
def cell(value,truncated=False):
    return {'value':value,'truncated':truncated}
result=table_result('table_preview',[None,None,None],[0])
assert result['columns']==['item label','amount'] and result['total_rows']==105 and result['shown_rows']==100 and result['truncated'],result
assert [row[0] for row in result['rows'][:3]]==[cell(None),cell('NULL'),cell('a\tb\nc')],result
assert result['rows'][0][1]==cell('9007199254740993.123456789'),result
assert table_result('table_preview',['0',None,None])=={
    'columns':['item label','amount'],'rows':[],'total_rows':0,'shown_rows':0,'truncated':False}
result=table_result('scalar_set')
assert result['columns']==['scalar_set'] and result['rows']==[[cell('11')],[cell(None)]],result
result=table_result('composite_set')
assert result['columns']==['id','label'] and result['rows']==[[cell('1'),cell('я'*8192,True)]] and result['truncated']
assert sql('SELECT count(*) FROM precise_types.guard')=='0'
sql('CREATE PROCEDURE precise_types.no_output(IN amount numeric) LANGUAGE plpgsql AS $$ BEGIN INSERT INTO precise_types.guard VALUES(amount); END; $$')
for _ in range(2):finish(wait(start(browse('no_output'),['1.23']),'paused'))
assert sql('SELECT count(*) FROM precise_types.guard')=='0'
cases=json.loads((Path(__file__).resolve().parents[3]/'core/tests/fixtures/scalar-values.json').read_text())
for case in cases:
    project={'schemaVersion':3,'model':{'name':'','tables':[]},'schema':'precise_types','name':'echo_'+case['type'],'kind':'function','returnType':case['type'],'parameters':[{'name':'input_value','type':case['type']}],'variables':[{'name':'result_value','type':case['type'],'initial':'(\ninput_value\n)'}],'blocks':[{'id':'return','type':'return','expression':'result_value'}]}
    payload={'connection_id':identity,'database':credentials['database'],'project':project};preview=post('/api/prepare',payload);post('/api/apply',{**payload,'preview_id':preview['preview_id']})
    details=browse(project['name']);assert details['arguments'][0]['supported'] and details['arguments'][0]['type_oid']==case['oid']
    for invalid in case['invalid'][:2]:start(details,[invalid],400)
    state=wait(start(details,[case['valid'][0]],project=project),'paused')
    assert state['graph_mapping'] and state['position']['block_id']=='return',state
    assert state['position']['line']==preview['sourceMap']['return']['bodyStart'],state
    variable=next(v['value'] for v in state['variables'] if v['name']=='result_value')
    assert variable==case['valid'][0],(case,variable)
    text=finish(state);expected=case['valid'][0].replace(' ','T') if case['type']=='timestamp' else case['valid'][0]
    assert expected in text,(case['type'],text)
    assert 'null' in finish(wait(start(details,[None]),'paused')).lower()
    assert sql('SELECT '+"'"+case['valid'][0]+"'::"+case['type']+'::text')==case['valid'][0]
sql("CREATE PROCEDURE precise_types.precise_proc(IN amount numeric, INOUT total bigint, OUT stamp timestamp, OUT id uuid, OUT exact_amount numeric, OUT day date) LANGUAGE plpgsql AS $$ BEGIN INSERT INTO precise_types.guard VALUES (amount); total := total + 1; stamp := timestamp '2026-11-18 01:02:03.123456'; id := '123e4567-e89b-12d3-a456-426614174000'; exact_amount := amount; day := date '2024-02-29'; END; $$")
text=finish(wait(start(browse('precise_proc'),['9007199254740993.1234567890123456789','9007199254740993',None,None,None,None]),'paused'))
assert '9007199254740994' in text and '2026-11-18 01:02:03.123456' in text and '123e4567-e89b-12d3-a456-426614174000' in text,text
assert '9007199254740993.1234567890123456789' in text and '2024-02-29' in text,text
text=finish(wait(start(browse('precise_proc'),[None,'2',None,None,None,None]),'paused'))
assert 'exact_amount = NULL' in text,text
assert sql('SELECT count(*) FROM precise_types.guard')=='0'
sql("CREATE TABLE precise_types.sums(amount bigint); INSERT INTO precise_types.sums VALUES (9223372036854775807),(1)")
assert sql('SELECT SUM(amount)::text FROM precise_types.sums')=='9223372036854775808'
sql("""
CREATE PROCEDURE precise_types.common_types(
    INOUT doc jsonb, INOUT stamp timestamptz, INOUT moment time,
    INOUT duration interval, INOUT payload bytea, INOUT ids integer[], OUT original_json json
) LANGUAGE plpgsql AS $$
BEGIN
    original_json := doc::json;
    INSERT INTO precise_types.guard VALUES (0);
END;
$$;
CREATE FUNCTION precise_types.array_count(ids integer[]) RETURNS integer LANGUAGE plpgsql AS $$
BEGIN
    RETURN cardinality(ids);
END;
$$;
""")
details=browse('common_types')
assert [arg['format'] for arg in details['arguments']]==['json','timestamptz','time','interval','hex','array','json']
assert all(arg['supported'] for arg in details['arguments'][:-1])
values=['{"amount":9007199254740993.123456789}','2026-11-20 14:30:00+10','12:34:56','2 days 03:04:05',r'\x48656c6c6f','{1,NULL,3}',None]
text=finish(wait(start(details,values),'paused'))
for expected in ('9007199254740993.123456789','original_json','12:34:56','2 days 03:04:05',r'\x48656c6c6f','{1,NULL,3}'):
    assert expected in text,text
assert '04:30:00+00' in text or '14:30:00+10' in text,text
text=finish(wait(start(details,['null',None,None,None,None,'{}',None]),'paused'))
assert 'doc = null' in text and 'ids = {}' in text,text
text=finish(wait(start(details,[None]*7),'paused'))
assert 'doc = NULL' in text and 'ids = NULL' in text,text
for invalid in ('{garbage}','[1,2]','{1,'):
    state=wait(start(browse('array_count'),[invalid]),'finished')
    assert state['outcome']['call']=='failed' and state['outcome']['rollback']=='confirmed',state
sql("""
CREATE SCHEMA other_types;
CREATE TYPE precise_types.status AS ENUM ('new', '', 'ready now');
CREATE TYPE other_types.status AS ENUM ('external');
CREATE DOMAIN precise_types.amount AS numeric CHECK (VALUE > 0);
CREATE DOMAIN precise_types.limited_amount AS precise_types.amount CHECK (VALUE < 10);
CREATE DOMAIN precise_types.active_status AS precise_types.status CHECK (VALUE <> 'new');
CREATE PROCEDURE precise_types.custom_types(
    INOUT state precise_types.active_status, INOUT amount precise_types.limited_amount,
    INOUT external other_types.status, OUT assigned precise_types.status
) LANGUAGE plpgsql AS $$
BEGIN
    assigned := state;
    INSERT INTO precise_types.guard VALUES (amount);
END;
$$;
""")
details=browse('custom_types')
assert all(arg['supported'] for arg in details['arguments'][:3])
assert details['arguments'][0]['enum_labels']==['new','','ready now']
assert details['arguments'][2]['enum_labels']==['external']
text=finish(wait(start(details,['ready now','1.234567890123456789','external',None]),'paused'))
assert 'state = ready now' in text and 'external = external' in text and 'assigned = ready now' in text,text
assert 'amount = 1.234567890123456789' in text,text
text=finish(wait(start(details,['','2','external',None]),'paused'))
assert 'assigned = ' in text and '<status>' not in text,text
start(details,['external','2','external',None],400)
for values in (['new','1','external',None],['ready now','-1','external',None],['ready now','12','external',None]):
    state=wait(start(details,values),'finished')
    assert state['outcome']['sqlstate']=='23514' and state['outcome']['rollback']=='confirmed',state
sql("""
CREATE TYPE precise_types.item AS (label text, old_label text, quantity integer);
ALTER TYPE precise_types.item DROP ATTRIBUTE old_label;
CREATE DOMAIN precise_types.valid_item AS precise_types.item CHECK ((VALUE).quantity >= 0);
CREATE DOMAIN precise_types.item_list AS precise_types.valid_item[] CHECK (cardinality(VALUE) < 5);
CREATE TABLE precise_types.catalog_item (label text, quantity integer);
CREATE FUNCTION precise_types.record_arg(arg precise_types.catalog_item) RETURNS text LANGUAGE plpgsql AS $$
BEGIN
    RETURN CASE WHEN arg IS NOT DISTINCT FROM NULL THEN 'record NULL'
                ELSE coalesce(arg.label, 'label NULL') || ':' || coalesce(arg.quantity::text, 'quantity NULL') END;
END;
$$;
CREATE PROCEDURE precise_types.collection_args(
    INOUT item precise_types.valid_item, INOUT items precise_types.item_list,
    INOUT statuses other_types.status[]
) LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO precise_types.guard VALUES (coalesce(item.quantity, 0));
END;
$$;
""")
details=browse('record_arg')
assert details['arguments'][0]['fields']==[{'name':'label','type_name':'text'},{'name':'quantity','type_name':'integer'}]
assert '":quantity NULL"' in finish(wait(start(details,['("",)']),'paused'))
assert 'label NULL:quantity NULL' in finish(wait(start(details,['(,)']),'paused'))
assert 'record NULL' in finish(wait(start(details,[None]),'paused'))
text=finish(wait(start(details,[r'("comma, \"quote\" and \\ slash",7)']),'paused'))
assert r'comma, \"quote\" and \\ slash:7' in text,text
sql('ALTER TABLE precise_types.catalog_item RENAME COLUMN quantity TO amount')
assert 'Поля составного аргумента изменились' in start(details,['(line,2)'],409)['error']
sql('ALTER TABLE precise_types.catalog_item RENAME COLUMN amount TO quantity')
details=browse('collection_args')
assert [arg['format'] for arg in details['arguments']]==['composite','array','array']
assert [field['name'] for field in details['arguments'][0]['fields']]==['label','quantity']
assert all(arg['supported'] for arg in details['arguments'])
text=finish(wait(start(details,['(first,2)','{"(second,3)",NULL}','{external,NULL}']),'paused'))
assert 'item = (first,2)' in text and 'items = {"(second,3)",NULL}' in text and 'statuses = {external,NULL}' in text,text
text=finish(wait(start(details,['(first,2)','{}',None]),'paused'))
assert 'items = {}' in text and 'statuses = NULL' in text,text
state=wait(start(details,['(first,2)','{"(invalid,-1)"}','{}']),'finished')
assert state['outcome']['sqlstate']=='23514' and state['outcome']['rollback']=='confirmed',state
types=post('/api/metadata',{'connection_id':identity,'database':credentials['database'],'schema':'precise_types','relation_oid':None,'types_only':True})['types']
status_type={'schema':'precise_types','name':'status'}
status_array={**status_type,'array':True}
assert status_type in types and status_array in types and {'schema':'precise_types','name':'item'} in types,types
project={
    'schemaVersion':3,'model':{'name':'','tables':[]},'schema':'precise_types','name':'constructor_types',
    'kind':'function','returnType':status_type,
    'parameters':[{'name':'statuses','type':status_array},{'name':'doc','type':'jsonb'}],
    'variables':[{'name':'result_value','type':status_type,'initial':'statuses[1]'}],
    'blocks':[{'id':'r','type':'return','expression':'result_value'}],
}
payload={'connection_id':identity,'database':credentials['database'],'project':project}
preview=post('/api/prepare',payload)
assert '"precise_types"."status"[]' in preview['signature'] and 'pg_catalog.jsonb' in preview['sql'],preview
post('/api/apply',{**payload,'preview_id':preview['preview_id']})
state=wait(start(browse(project['name']),['{new}','{"x":1}'],project=project),'paused')
assert state['graph_mapping'] and state['position']['block_id']=='r',state
assert 'new' in finish(state)
missing=copy.deepcopy(project)
missing['variables'][0]['type']={'schema':'precise_types','name':'missing_type'}
assert 'не найден' in post('/api/prepare',{**payload,'project':missing},400)['error']
sql("CREATE TYPE precise_types.ephemeral_status AS ENUM ('ready')")
stale=copy.deepcopy(project)
stale['name']='constructor_stale_type'
stale['variables'].append({'name':'old_type','type':{'schema':'precise_types','name':'ephemeral_status'}})
stale_payload={**payload,'project':stale}
preview=post('/api/prepare',stale_payload)
sql("DROP TYPE precise_types.ephemeral_status; CREATE TYPE precise_types.ephemeral_status AS ENUM ('ready')")
post('/api/apply',{**stale_payload,'preview_id':preview['preview_id']},409)
sql("""
CREATE FUNCTION precise_types.with_defaults(
    base integer, middle integer DEFAULT 7, "last p" text DEFAULT 'default'
) RETURNS text LANGUAGE plpgsql AS $$
BEGIN
    RETURN base || ':' || coalesce(middle::text, 'null') || ':' || coalesce("last p", 'null');
END;
$$;
CREATE FUNCTION precise_types.with_defaults() RETURNS text
LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'wrong overload'; END; $$;
CREATE PROCEDURE precise_types.proc_with_defaults(
    OUT result text, INOUT middle integer DEFAULT 12, IN suffix text DEFAULT 'z'
) LANGUAGE plpgsql AS $$
BEGIN
    result := coalesce(middle::text,'null') || ':' || suffix;
    INSERT INTO precise_types.guard VALUES (middle);
END;
$$;
CREATE FUNCTION precise_types.only_defaults(integer DEFAULT 20, integer DEFAULT 22) RETURNS integer
LANGUAGE plpgsql AS $$ BEGIN RETURN $1 + $2; END; $$;
CREATE FUNCTION precise_types.default_only_type(host inet DEFAULT '192.0.2.1') RETURNS text
LANGUAGE plpgsql AS $$ BEGIN RETURN host::text; END; $$;
CREATE FUNCTION precise_types.overload_guard(n integer DEFAULT 1) RETURNS integer
LANGUAGE plpgsql AS $$ BEGIN INSERT INTO precise_types.guard VALUES (100); RETURN n; END; $$;
CREATE FUNCTION precise_types.overload_guard() RETURNS integer
LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'wrong overload'; END; $$;
""")
def overload(name,signature):
    routines=post('/api/catalog',{'connection_id':identity,'database':credentials['database'],'schema':'precise_types'})['routines']
    routine=next(r for r in routines if r['name']==name and signature in r['signature'])
    return post('/api/catalog',{'connection_id':identity,'database':credentials['database'],'schema':'precise_types','routine_oid':routine['oid']})['selected']
details=overload('with_defaults','base integer')
assert [arg['has_default'] for arg in details['arguments']]==[False,True,True]
assert '3:7:default' in finish(wait(start(details,['3',None,None],defaults=[1,2]),'paused'))
assert '3:7:given' in finish(wait(start(details,['3',None,'given'],defaults=[1]),'paused'))
assert '3:null:null' in finish(wait(start(details,['3',None,None]),'paused'))
for indexes in ([0],[99],[1,9999]):
    start(details,['3',None,None],400,defaults=indexes)
start(details,['3','11',None],400,defaults=[1])
details=browse('proc_with_defaults')
assert [arg['has_default'] for arg in details['arguments']]==[False,True,True]
assert 'result = 12:given' in finish(wait(start(details,[None,None,'given'],defaults=[1]),'paused'))
assert 'result = 12:z' in finish(wait(start(details,[None,None,None],defaults=[1,2]),'paused'))
start(details,[None,None,None],400,defaults=[0,1])
details=browse('only_defaults')
assert '42' in finish(wait(start(details,[None,None],defaults=[0,1]),'paused'))
start(details,[None,'12'],400,defaults=[0])
details=browse('default_only_type')
assert not details['arguments'][0]['supported'] and details['arguments'][0]['has_default']
assert '192.0.2.1' in finish(wait(start(details,[None],defaults=[0]),'paused'))
start(details,['192.0.2.2'],400)
details=overload('overload_guard','integer')
state=wait(start(details,[None],defaults=[0]),'finished')
assert state['outcome']['call']=='not_started' and 'перегрузки' in state['outcome']['text'],state
assert '17' in finish(wait(start(details,['17']),'paused'))
assert sql('SELECT count(*) FROM precise_types.guard')=='0'
post('/api/disconnect',{})
print('HTTP types and defaults: scalar and custom types, NULL, omitted arguments, named CALL, overload guard and rollback passed.')
