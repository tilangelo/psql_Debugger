"""DDL проходит только на временной базе стенда. Никакие пользовательские базы не используются."""
import json,os,re,subprocess,urllib.request,urllib.error,copy
base=os.environ['PGDEBUG_WEB_TEST_URL']
html=urllib.request.urlopen(base).read().decode()
token=re.search(r'name="pg-debugger-token" content="([a-f0-9]+)"',html).group(1)
password=os.environ['PGDEBUG_GUI_TEST_PASSWORD']
headers={'Content-Type':'application/json','x-pg-debugger-token':token}
def post(path,body,status=200):
    req=urllib.request.Request(base+path,data=json.dumps(body).encode(),headers=headers)
    try:
        with urllib.request.urlopen(req,timeout=15) as r: code,text=r.status,r.read().decode()
    except urllib.error.HTTPError as error:code,text=error.code,error.read().decode()
    assert code==status,(path,code,status,text)
    assert password not in text
    return json.loads(text)
credentials={'host':'127.0.0.1','port':os.environ['PGDEBUG_GUI_TEST_PORT'],'database':'gui_plugin_test','user':'debugger_test','password':password}
identity=post('/api/connect',credentials)['connection_id']
def sql(command):
    env={**os.environ,'PGPASSWORD':password}
    result=subprocess.run([os.environ.get('PGDEBUG_TEST_PSQL','/usr/lib/postgresql/17/bin/psql'),'-h','127.0.0.1','-p',credentials['port'],'-U','debugger_test','-d',credentials['database'],'-XAt','-v','ON_ERROR_STOP=1','-c',command],env=env,text=True,capture_output=True)
    assert result.returncode==0,result.stderr
    return result.stdout.strip()
project={'schemaVersion':1,'kind':'function','schema':'public','name':'web_apply_test','returnType':'integer','parameters':[{'name':'input_value','type':'integer'}],'variables':[{'name':'running_total','type':'integer','initial':'input_value'}],'blocks':[{'id':'a','type':'assign','target':'running_total','expression':'running_total + 1'},{'id':'b','type':'assign','target':'running_total','expression':'running_total * 2'},{'id':'r','type':'return','expression':'running_total'}]}
def payload(p):return {'connection_id':identity,'database':credentials['database'],'project':p}
def prepare(p,status=200):return post('/api/prepare',payload(p),status)
def apply(p,preview,status=200):return post('/api/apply',{**payload(p),'preview_id':preview['preview_id']},status)
signature='public.web_apply_test(integer)'
preview=prepare(project)
assert preview['action']=='create' and preview['before'] is None
assert sql(f"SELECT to_regprocedure('{signature}') IS NULL")=='t','Preview persisted a routine'
stale_preview=preview
preview=prepare(project)
apply(project,stale_preview,409)
apply(project,preview);assert sql('SELECT public.web_apply_test(3)')=='8'
apply(project,preview,409)
# Проверка замены откатывается, а права сохраняются после применения.
sql('REVOKE EXECUTE ON FUNCTION public.web_apply_test(integer) FROM PUBLIC; GRANT EXECUTE ON FUNCTION public.web_apply_test(integer) TO gui_reader')
updated=copy.deepcopy(project);updated['blocks'][0]['expression']='running_total + 2'
preview=prepare(updated);assert preview['before'] and preview['action']=='replace'
assert sql('SELECT public.web_apply_test(3)')=='8'
apply(updated,preview);assert sql('SELECT public.web_apply_test(3)')=='10'
assert sql("SELECT has_function_privilege('gui_reader','public.web_apply_test(integer)','EXECUTE')")=='t'
# Редактирование после проверки не позволяет применить старый предпросмотр.
preview=prepare(updated);apply(project,preview,409)
# Изменение в стороннем клиенте между проверкой и применением обнаруживается.
preview=prepare(updated)
sql('CREATE OR REPLACE FUNCTION public.web_apply_test(input_value integer) RETURNS integer LANGUAGE plpgsql AS $$ BEGIN RETURN 500; END; $$')
apply(updated,preview,409);assert sql('SELECT public.web_apply_test(3)')=='500'
# Ошибка SQL не сохраняет новую функцию.
broken=copy.deepcopy(project);broken['name']='web_bad_test';broken['blocks'].insert(0,{'id':'bad','type':'sql','sql':'THIS IS NOT VALID PLPGSQL;'})
prepare(broken,400);assert sql("SELECT to_regprocedure('public.web_bad_test(integer)') IS NULL")=='t'
# Тело не исполняется при проверке и применении.
sql('CREATE TABLE public.web_apply_guard(value integer)')
guard=copy.deepcopy(project);guard['name']='web_guard_test';guard['blocks'].insert(0,{'id':'guard','type':'sql','sql':'INSERT INTO public.web_apply_guard VALUES (99);'})
preview=prepare(guard);assert sql('SELECT count(*) FROM public.web_apply_guard')=='0'
apply(guard,preview);assert sql('SELECT count(*) FROM public.web_apply_guard')=='0'
assert sql('SELECT public.web_guard_test(3)')=='8';assert sql('SELECT count(*) FROM public.web_apply_guard')=='1'
# Процедура создаётся отдельно, CALL выполняется только этой проверкой.
proc=copy.deepcopy(project);proc['name']='web_proc_test';proc['kind']='procedure';proc['blocks'][-1]={'id':'notice','type':'sql','sql':"RAISE NOTICE 'value=%', running_total;"}
apply(proc,prepare(proc));sql('CALL public.web_proc_test(3)')
# Особые свойства не должны сбрасываться неявно.
sql('ALTER FUNCTION public.web_apply_test(integer) SECURITY DEFINER')
prepare(project,400);assert sql("SELECT prosecdef FROM pg_proc WHERE oid='public.web_apply_test(integer)'::regprocedure")=='t'
post('/api/disconnect',{})
print('HTTP apply: preview rollback, creation, replacement, ACL, stale project/definition, syntax error, body not called, procedure and protected settings passed.')
