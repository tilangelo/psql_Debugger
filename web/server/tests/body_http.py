"""Изменение только тела сохраняет свойства, ACL и сигнатуру существующего объекта."""
import json,os,re,subprocess,urllib.request,urllib.error,time
base=os.environ['PGDEBUG_WEB_TEST_URL'];password=os.environ['PGDEBUG_GUI_TEST_PASSWORD']
page=urllib.request.urlopen(base).read().decode();token=re.search(r'name="pg-debugger-token" content="([a-f0-9]+)"',page).group(1)
headers={'Content-Type':'application/json','x-pg-debugger-token':token}
credentials={'host':'127.0.0.1','port':os.environ['PGDEBUG_GUI_TEST_PORT'],'database':'gui_plugin_test','user':'debugger_test','password':password}
def post(path,body,status=200):
    try:
        with urllib.request.urlopen(urllib.request.Request(base+path,data=json.dumps(body).encode(),headers=headers),timeout=15) as response:code,text=response.status,response.read().decode()
    except urllib.error.HTTPError as err:code,text=err.code,err.read().decode()
    assert code==status,(path,code,status,text)
    assert password not in text
    return json.loads(text)
def sql(command):
    return subprocess.check_output([os.environ.get('PGDEBUG_TEST_PSQL','/usr/lib/postgresql/17/bin/psql'),'-h','127.0.0.1','-p',credentials['port'],'-U','debugger_test','-d',credentials['database'],'-XAt','-v','ON_ERROR_STOP=1','-c',command],env={**os.environ,'PGPASSWORD':password},text=True).strip()
sql("""CREATE SCHEMA body_checks; CREATE TABLE body_checks.guard(value integer);
CREATE FUNCTION body_checks.calculate(value integer DEFAULT 7) RETURNS integer LANGUAGE plpgsql STABLE STRICT SECURITY DEFINER PARALLEL SAFE COST 321 SET search_path=pg_catalog,body_checks AS $$BEGIN RETURN value + 1; END;$$;
COMMENT ON FUNCTION body_checks.calculate(integer) IS 'Keep this comment';
REVOKE ALL ON FUNCTION body_checks.calculate(integer) FROM PUBLIC;
GRANT USAGE ON SCHEMA body_checks TO gui_reader;
GRANT EXECUTE ON FUNCTION body_checks.calculate(integer) TO gui_reader;
CREATE PROCEDURE body_checks.process(INOUT counter integer, OUT result text) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$BEGIN counter:=counter+1; result:=counter::text; END;$$;
CREATE FUNCTION body_checks.side_effect() RETURNS integer LANGUAGE plpgsql AS $$BEGIN INSERT INTO body_checks.guard VALUES(1); RETURN 1; END;$$;""")
catalog=post('/api/connect',credentials);identity=catalog['connection_id']
def selected(name):
    request={'connection_id':identity,'database':credentials['database'],'schema':'body_checks'}
    routine=next(r for r in post('/api/catalog',request)['routines'] if r['name']==name)
    return post('/api/catalog',{**request,'routine_oid':routine['oid']})['selected']
def payload(details,body):return {'connection_id':identity,'database':credentials['database'],'routine_oid':details['oid'],'expected_definition':details['definition'],'body':body}
def attributes(oid):return sql(f"SELECT row(p.oid,p.proowner,p.proacl,p.prosecdef,p.proisstrict,p.provolatile,p.proparallel,p.proconfig,p.pronargdefaults,p.proargdefaults::text,p.proargtypes,p.proallargtypes,p.proargmodes,p.procost,p.proretset,p.proleakproof,p.prosupport,pg_catalog.obj_description(p.oid,'pg_proc'))::text FROM pg_proc p WHERE p.oid={oid}")
details=selected('calculate');before=attributes(details['oid'])
request=payload(details,'BEGIN\n -- $pgdebug_body$; DROP TABLE body_checks.guard;\n RETURN value+2;\nEND;')
preview=post('/api/body/prepare',request)
assert sql('SELECT body_checks.calculate()')=='8','Preview persisted a definition'
assert 'SECURITY DEFINER' in preview['after'] and 'COST 321' in preview['after'] and 'DEFAULT 7' in preview['after']
stale_preview=preview
preview=post('/api/body/prepare',request)
post('/api/body/apply',{**request,'preview_id':stale_preview['preview_id']},409)
post('/api/body/apply',{**request,'preview_id':preview['preview_id']})
assert sql('SELECT body_checks.calculate()')=='9' and attributes(details['oid'])==before
assert sql("SELECT count(*) FROM pg_class WHERE oid='body_checks.guard'::regclass")=='1'
post('/api/body/apply',{**request,'preview_id':preview['preview_id']},409)
details=selected('calculate')
post('/api/body/prepare',payload(details,'BEGIN RETURN ; END;'),400)
located=post('/api/body/prepare',payload(details,'BEGIN\n -- 😀 кириллица\n RETURN ;\nEND;'),400)
assert located['location']=={'line':3,'column':9},located
assert sql('SELECT body_checks.calculate()')=='9'
stale=payload(details,'BEGIN RETURN value+3; END;');preview=post('/api/body/prepare',stale)
sql('ALTER FUNCTION body_checks.calculate(integer) COST 322')
post('/api/body/apply',{**stale,'preview_id':preview['preview_id']},409)
assert sql('SELECT body_checks.calculate()')=='9'
post('/api/body/prepare',stale,409)
details=selected('side_effect');request=payload(details,'BEGIN INSERT INTO body_checks.guard VALUES(2); RETURN 2; END;')
preview=post('/api/body/prepare',request);assert sql('SELECT count(*) FROM body_checks.guard')=='0'
post('/api/body/apply',{**request,'preview_id':preview['preview_id']});assert sql('SELECT count(*) FROM body_checks.guard')=='0'
# Изменение черновика после проверки не применяется.
details=selected('side_effect');request=payload(details,'BEGIN RETURN 3; END;');preview=post('/api/body/prepare',request)
post('/api/body/apply',{**request,'body':'BEGIN RETURN 4; END;','preview_id':preview['preview_id']},409)
assert 'VALUES(2)' in selected('side_effect')['source']
# OUT/INOUT и свойства процедуры сохраняются.
details=selected('process');before=attributes(details['oid']);request=payload(details,"BEGIN counter:=counter+2; result:=counter::text; END;")
preview=post('/api/body/prepare',request);post('/api/body/apply',{**request,'preview_id':preview['preview_id']})
assert attributes(details['oid'])==before and sql('CALL body_checks.process(3,NULL)')=='5|5'
# Обычного EXECUTE недостаточно для редактирования чужого тела.
details=selected('calculate');reader=post('/api/connect',{**credentials,'user':'gui_reader'})
post('/api/body/prepare',{**payload(details,'BEGIN RETURN 0; END;'),'connection_id':reader['connection_id']},400)
catalog=post('/api/connect',credentials);identity=catalog['connection_id']
# Применённое тело доступно той же отладке; изменение определения не выполняет вызов.
details=selected('side_effect');state=post('/api/debug/start',{'connection_id':identity,'database':credentials['database'],'schema':'body_checks','routine_oid':details['oid'],'expected_definition':details['definition'],'values':[],'breakpoints':[],'project':None})
def wait(state,phase):
    until=time.monotonic()+15
    while state['phase']!=phase:
        assert time.monotonic()<until,state
        time.sleep(.1);state=post('/api/debug/poll',{'session_id':state['session_id']})
    return state
state=wait(state,'paused');state=post('/api/debug/command',{'session_id':state['session_id'],'revision':state['revision'],'action':{'command':'continue'}})
state=wait(state,'finished');assert state['outcome']['call']=='succeeded' and state['outcome']['rollback']=='confirmed'
assert sql('SELECT count(*) FROM body_checks.guard')=='0'
post('/api/disconnect',{})
print('HTTP body editor: preview rollback, explicit apply, attributes/ACL/defaults/OUT preservation, quoted body, conflicts, owner checks, no implicit invocation and real debug rollback passed.')
