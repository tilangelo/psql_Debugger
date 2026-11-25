"""Настоящий отладочный цикл через HTTP, включая heartbeat и откат."""
import copy,json,os,re,subprocess,time,urllib.request,urllib.error
base=os.environ['PGDEBUG_WEB_TEST_URL'];password=os.environ['PGDEBUG_GUI_TEST_PASSWORD']
page=urllib.request.urlopen(base).read().decode();token=re.search(r'name="pg-debugger-token" content="([a-f0-9]+)"',page).group(1)
headers={'Content-Type':'application/json','x-pg-debugger-token':token}
def post(path,body,status=200):
    try:
        with urllib.request.urlopen(urllib.request.Request(base+path,data=json.dumps(body).encode(),headers=headers),timeout=15) as r:code,text=r.status,r.read().decode()
    except urllib.error.HTTPError as e:code,text=e.code,e.read().decode()
    assert code==status,(path,code,status,text)
    assert password not in text
    return json.loads(text)
credentials={'host':'127.0.0.1','port':os.environ['PGDEBUG_GUI_TEST_PORT'],'database':'gui_plugin_test','user':'debugger_test','password':password}
identity=post('/api/connect',credentials)['connection_id']
def sql(command):
    result=subprocess.run([os.environ.get('PGDEBUG_TEST_PSQL','/usr/lib/postgresql/17/bin/psql'),'-h','127.0.0.1','-p',credentials['port'],'-U','debugger_test','-d','gui_plugin_test','-XAt','-v','ON_ERROR_STOP=1','-c',command],env={**os.environ,'PGPASSWORD':password},capture_output=True,text=True,timeout=10)
    assert result.returncode==0,result.stderr
    return result.stdout.strip()
def browse(name):
    data=post('/api/catalog',{'connection_id':identity,'database':'gui_plugin_test','schema':'public'})
    routine=next(r for r in data['routines'] if r['name']==name)
    return post('/api/catalog',{'connection_id':identity,'database':'gui_plugin_test','schema':'public','routine_oid':routine['oid']})['selected']
project={'schemaVersion':1,'kind':'function','schema':'public','name':'web_debug_test','returnType':'integer','parameters':[{'name':'input_value','type':'integer'}],'variables':[{'name':'running_total','type':'integer','initial':'input_value'}],'blocks':[{'id':'a','type':'assign','target':'running_total','expression':'running_total + 1'},{'id':'b','type':'assign','target':'running_total','expression':'running_total * 2'},{'id':'r','type':'return','expression':'running_total'}]}
def install(p):
    body={'connection_id':identity,'database':'gui_plugin_test','project':p}
    preview=post('/api/prepare',body)
    post('/api/apply',{**body,'preview_id':preview['preview_id']})
    return preview
preview=install(project);selected=browse(project['name'])
def start(details,values=None,points=None,p=None,status=200,nested=None,trigger=None):
    return post('/api/debug/start',{'connection_id':identity,'database':'gui_plugin_test','schema':'public','routine_oid':details['oid'],'expected_definition':details['definition'],'values':values if values is not None else ['3'],'breakpoints':points or [],'nested_breakpoints':nested or [],'project':p,'trigger':trigger},status)
def poll(state):return post('/api/debug/poll',{'session_id':state['session_id']})
def wait(state,phase):
    deadline=time.monotonic()+15
    while time.monotonic()<deadline:
        if state['phase']==phase:return state
        assert state['phase']!='finished' or phase=='finished',state
        time.sleep(.1);state=poll(state)
    raise AssertionError(('Timeout waiting for '+phase,state))
def command(state,action,status=200):return post('/api/debug/command',{'session_id':state['session_id'],'revision':state['revision'],'action':action},status)
def value(state,name):return next(v['value'] for v in state['variables'] if v['name']==name)
def finished(state,call):
    state=wait(state,'finished');assert state['outcome']['call']==call,state
    assert state['outcome']['rollback']=='confirmed',state
    return state
state=wait(start(selected,p=project),'paused')
assert state['graph_mapping'] and state['position']['block_id']=='a',state
assert state['position']['line']==preview['sourceMap']['a']['bodyStart']==5
assert selected['source'].splitlines()[state['position']['line']-1].strip().startswith('"running_total" :=')
assert value(state,'running_total')=='3'
post('/api/connect',credentials,409);post('/api/disconnect',{},409)
assert post('/api/debug/current',{})['session_id']==state['session_id']
assert state['pause_id']==1 and state['stops'][0]['line']==5
old=copy.deepcopy(state);state=wait(command(state,{'command':'next'}),'paused')
assert state['position']['block_id']=='b' and value(state,'running_total')=='4'
assert state['pause_id']==2 and len(state['stops'])==2
command(old,{'command':'next'},409)
line=preview['sourceMap']['r']['bodyStart']
state=command(state,{'command':'breakpoint','line':line,'enabled':True})
assert state['pause_id']==2, 'Breakpoint update is not a pause'
state=wait(command(state,{'command':'continue'}),'paused')
assert state['pause_id']==3 and len(state['stops'])==3
assert state['position']['line']==line and value(state,'running_total')=='8'
state=command(state,{'command':'breakpoint','line':line,'enabled':False})
result=finished(command(state,{'command':'continue'}),'succeeded')
assert '8' in result['outcome']['text']
assert result['outcome']['scalar']=='8'
sql("CREATE FUNCTION public.web_debug_long_result() RETURNS text LANGUAGE plpgsql AS $$ BEGIN RETURN repeat('x', 200); END; $$")
long_result=wait(start(browse('web_debug_long_result'),values=[]),'paused')
long_result=finished(command(long_result,{'command':'continue'}),'succeeded')['outcome']
quoted = json.dumps('x'*200)
assert long_result['scalar']==quoted[:120]+'…', ascii(long_result['scalar'])
assert quoted in long_result['text']

# Вход во вложенный вызов и просмотр родителя без возобновления исполнения.
sql("""
CREATE FUNCTION public.web_debug_child(child_input integer) RETURNS integer LANGUAGE plpgsql AS $$
DECLARE child_value integer := child_input + 10;
BEGIN
    child_value := child_value * 2;
    RETURN child_value;
END;
$$;
CREATE FUNCTION public.web_debug_parent(parent_input integer) RETURNS integer LANGUAGE plpgsql AS $$
DECLARE parent_value integer := parent_input + 1;
BEGIN
    parent_value := public.web_debug_child(parent_value);
    RETURN parent_value + 100;
END;
$$;
""")
parent=browse('web_debug_parent')
state=wait(start(parent),'paused')
assert state['frame']['oid']==parent['oid'] and value(state,'parent_value')=='4'
state=wait(command(state,{'command':'step_into'}),'paused')
assert state['position']['oid']!=parent['oid'] and state['position']['line']==4,state
assert 'child_value := child_value * 2' in state['frame']['source'],state
assert state['frame']['level']==0 and value(state,'child_value')=='14'
pause,position,stops=state['pause_id'],state['position'],state['stops']
command(state,{'command':'select_frame','level':99},400)
def choose_frame(state,level):
    state=command(state,{'command':'select_frame','level':level})
    deadline=time.monotonic()+5
    while state.get('frame_pending'):
        assert time.monotonic()<deadline and state['phase']=='paused',state
        time.sleep(.05);state=poll(state)
    assert state['frame']['level']==level,state
    return state
state=choose_frame(state,1)
assert state['frame']['oid']==parent['oid'] and value(state,'parent_value')=='4'
assert (state['pause_id'],state['position'],state['stops'])==(pause,position,stops)
assert post('/api/debug/current',{})['frame']['level']==1
state=choose_frame(state,0)
assert value(state,'child_value')=='14'
state=choose_frame(state,1)
state=wait(command(state,{'command':'next'}),'paused')
assert state['frame']['level']==0 and value(state,'child_value')=='28',state
state=wait(command(state,{'command':'next'}),'paused')
assert state['position']['oid']==parent['oid'] and value(state,'parent_value')=='28',state
assert '128' in finished(command(state,{'command':'continue'}),'succeeded')['outcome']['text']
state=wait(start(parent),'paused')
command(state,{'command':'step_out'},400)
state=wait(command(state,{'command':'next'}),'paused')
assert state['frame']['oid']==parent['oid'] and value(state,'parent_value')=='28'
assert '128' in finished(command(state,{'command':'continue'}),'succeeded')['outcome']['text']
state=wait(start(parent),'paused')
state=wait(command(state,{'command':'step_into'}),'paused')
state=choose_frame(state,1)
pause=state['pause_id']
state=wait(command(state,{'command':'step_out'}),'paused')
assert state['position']['oid']==parent['oid'] and state['position']['line']==5,state
assert state['frame']['level']==0 and value(state,'parent_value')=='28',state
assert state['pause_id']==pause+1 and len(state['stops'])==pause+1,state
assert '128' in finished(command(state,{'command':'continue'}),'succeeded')['outcome']['text']
print('HTTP nested debugging: step over/into, executing position, parent/child source and variables, reload and rollback passed.')
# Точки в разных объектах могут иметь один номер. Точка вложенного объекта
# остаётся активной при просмотре родителя и при следующем вызове той же функции.
sql("""
CREATE FUNCTION public.web_debug_repeat(n integer) RETURNS integer LANGUAGE plpgsql AS $$
DECLARE result integer := n;
BEGIN
    result := public.web_debug_child(result);
    result := public.web_debug_child(result);
    result := public.web_debug_child(result);
    RETURN result;
END;
$$;
CREATE TABLE public.web_nested_changes(value integer);
CREATE PROCEDURE public.web_inner_proc(n integer) LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO public.web_nested_changes VALUES (n);
    INSERT INTO public.web_nested_changes VALUES (n + 1);
END;
$$;
CREATE PROCEDURE public.web_outer_proc(n integer) LANGUAGE plpgsql AS $$
BEGIN
    CALL public.web_inner_proc(n);
    NULL;
END;
$$;
""")
def nested_points(state,oid,lines,enabled=True,status=200):
    response=command(state,{'command':'nested_breakpoints','oid':oid,'lines':lines,'enabled':enabled},status)
    if status!=200:return response
    deadline=time.monotonic()+5
    while response.get('frame_pending'):
        assert time.monotonic()<deadline and response['phase']=='paused',response
        time.sleep(.05);response=poll(response)
    assert response['pause_id']==state['pause_id'] and response['position']==state['position'],response
    return response

repeat=browse('web_debug_repeat')
child=browse('web_debug_child')
state=wait(start(repeat),'paused')
nested_points(state,child['oid'],[5],status=400)
state=command(state,{'command':'breakpoint','line':5,'enabled':True})
state=wait(command(state,{'command':'step_into'}),'paused')
assert state['position']['oid']==child['oid'] and state['position']['line']==4,state
nested_points(state,repeat['oid'],[5],status=400)
state=nested_points(state,child['oid'],[2_000_000])
assert state['warning'] and not state['nested_breakpoints'],state
state=nested_points(state,child['oid'],[5])
assert not state['warning'] and state['nested_breakpoints'][0]['lines']==[5],state
state=nested_points(state,child['oid'],[3])
assert state['nested_breakpoints'][0]['lines']==[4,5],state
state=choose_frame(state,1)
assert post('/api/debug/current',{})['nested_breakpoints']==state['nested_breakpoints']
state=wait(command(state,{'command':'step_out'}),'paused')
assert state['position']['oid']==child['oid'] and state['position']['line']==5,state
state=wait(command(state,{'command':'continue'}),'paused')
assert state['position']['oid']==repeat['oid'] and state['position']['line']==5,state
state=nested_points(state,child['oid'],[5],False)
assert state['breakpoints']==[5] and state['nested_breakpoints'][0]['lines']==[4],state
state=wait(command(state,{'command':'continue'}),'paused')
assert state['position']['oid']==child['oid'] and state['position']['line']==4,state
state=nested_points(state,child['oid'],[4],False)
assert not state['nested_breakpoints'] and state['breakpoints']==[5],state
result=finished(command(state,{'command':'continue'}),'succeeded')
assert '164' in result['outcome']['text'],result
state=wait(start(browse('web_outer_proc')),'paused')
state=wait(command(state,{'command':'step_into'}),'paused')
assert 'web_inner_proc' in state['position']['target'],state
state=nested_points(state,state['frame']['oid'],[4])
state=wait(command(state,{'command':'continue'}),'paused')
assert state['position']['line']==4 and sql('SELECT count(*) FROM public.web_nested_changes')=='0',state
result=finished(command(state,{'command':'continue'}),'succeeded')
assert not result['nested_breakpoints'] and sql('SELECT count(*) FROM public.web_nested_changes')=='0'
print('HTTP nested breakpoints: independent lines, entry normalization, repeated calls, frame inspection, removal and procedure rollback passed.')

def planned(details,lines,schema='public'):
    return {'schema':schema,'oid':details['oid'],'expected_definition':details['definition'],'lines':lines}
start(parent,nested=[planned(parent,[4])],status=400)
start(parent,nested=[planned(child,[0])],status=400)
start(parent,nested=[planned(child,[5]),planned(child,[4])],status=400)
start(parent,nested=[planned(child,[5]*33)],status=400)
start(parent,nested=[{**planned(child,[5]),'expected_definition':'stale'}],status=409)
start(parent,nested=[planned(child,[5],'wrong_schema')],status=409)
state=wait(start(parent,nested=[planned(child,[5])]),'paused')
assert state['pause_id']==1 and state['position']['oid']==child['oid'] and state['position']['line']==5,state
assert state['nested_breakpoints'][0]['lines']==[5] and 'web_debug_child' in state['nested_breakpoints'][0]['name'],state
assert value(state,'child_value')=='28' and sql('SELECT public.web_debug_child(1)')=='22',state
state=nested_points(state,child['oid'],[5],False)
assert not state['nested_breakpoints'],state
assert '128' in finished(command(state,{'command':'continue'}),'succeeded')['outcome']['text']
state=wait(start(parent,points=[4],nested=[planned(child,[3])]),'paused')
assert state['position']['oid']==parent['oid'] and state['nested_breakpoints'][0]['lines']==[4],state
state=wait(command(state,{'command':'continue'}),'paused')
assert state['position']['oid']==child['oid'] and state['position']['line']==4,state
finished(command(state,{'command':'continue'}),'succeeded')
inner_proc=browse('web_inner_proc')
state=wait(start(browse('web_outer_proc'),nested=[planned(inner_proc,[4])]),'paused')
assert state['pause_id']==1 and state['position']['oid']==inner_proc['oid'] and state['position']['line']==4,state
finished(command(state,{'command':'continue'}),'succeeded')
assert sql('SELECT count(*) FROM public.web_nested_changes')=='0'
result=finished(start(browse('debug_smoke'),nested=[planned(child,[5])]),'succeeded')
assert result['pause_id']==0 and '8' in result['outcome']['text'],result
print('HTTP prepared nested points: validation, first function/procedure call, entry mapping, target-only scope, removal and rollback passed.')

sql("""
CREATE FUNCTION public.web_step_out_rec(n integer) RETURNS integer LANGUAGE plpgsql AS $$
BEGIN
    IF n > 0 THEN
        n := public.web_step_out_rec(n - 1);
    END IF;
    n := n + 1;
    RETURN n;
END;
$$;
CREATE FUNCTION public.web_step_out_loop(n integer) RETURNS integer LANGUAGE plpgsql AS $$
BEGIN
    FOR i IN 1..n LOOP
        PERFORM i;
    END LOOP;
    RETURN n;
END;
$$;
CREATE FUNCTION public.web_step_out_limit(n integer) RETURNS integer LANGUAGE plpgsql AS $$
BEGIN
    RETURN public.web_step_out_loop(n);
END;
$$;
CREATE FUNCTION public.web_step_out_sleep(n integer) RETURNS integer LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO public.web_nested_changes VALUES (n);
    PERFORM pg_sleep(30);
    RETURN n;
END;
$$;
CREATE FUNCTION public.web_step_out_wait(n integer) RETURNS integer LANGUAGE plpgsql AS $$
BEGIN
    RETURN public.web_step_out_sleep(n);
END;
$$;
""")
rec=browse('web_step_out_rec')
state=wait(start(rec,values=['2']),'paused')
state=wait(command(state,{'command':'next'}),'paused')
state=wait(command(state,{'command':'step_into'}),'paused')
assert len(state['stack'])==2 and state['frame']['oid']==rec['oid'],state
state=wait(command(state,{'command':'step_out'}),'paused')
assert len(state['stack'])==1 and state['position']['line']==6 and value(state,'n')=='2',state
assert '3' in finished(command(state,{'command':'continue'}),'succeeded')['outcome']['text']
state=wait(start(browse('web_step_out_limit'),values=['1000']),'paused')
state=wait(command(state,{'command':'step_into'}),'paused')
for _ in range(2):
    pause=state['pause_id']
    state=wait(command(state,{'command':'step_out'}),'paused')
    assert len(state['stack'])==2 and state['pause_id']==pause+1 and '16' in state['warning'],state
finished(command(state,{'command':'continue'}),'succeeded')
state=wait(start(browse('web_step_out_wait')),'paused')
state=wait(command(state,{'command':'step_into'}),'paused')
state=command(state,{'command':'step_out'})
deadline=time.monotonic()+8
while sql("SELECT count(*) FROM pg_stat_activity WHERE query LIKE '%web_step_out_wait%' AND wait_event='PgSleep'")=='0':
    assert time.monotonic()<deadline,state
    time.sleep(.05)
result=finished(command(state,{'command':'stop'}),'failed')
assert result['outcome']['cancellation_requested'] and sql('SELECT count(*) FROM public.web_nested_changes')=='0'
print('HTTP step out: caller, deepest frame, user breakpoint, recursion, bounded steps, independent cancel and rollback passed.')
sql("""
CREATE TABLE public."trigger sandbox" (
    tenant integer, "ticket id" integer, label text, payload jsonb DEFAULT '{}',
    derived integer GENERATED ALWAYS AS ("ticket id" + 1) STORED,
    PRIMARY KEY (tenant,"ticket id")
);
INSERT INTO public."trigger sandbox" (tenant,"ticket id",label) VALUES (7,11,'original'),(8,11,'other');
CREATE TABLE public.trigger_audit(operation text, label text);
CREATE FUNCTION public.web_debug_trigger() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE operation_seen text := TG_OP; label_seen text;
BEGIN
    IF TG_OP = 'DELETE' THEN
        label_seen := OLD.label;
    ELSE
        label_seen := NEW.label;
    END IF;
    INSERT INTO public.trigger_audit VALUES (operation_seen,label_seen);
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER "debug target" BEFORE INSERT OR UPDATE OF label OR DELETE ON public."trigger sandbox"
FOR EACH ROW EXECUTE FUNCTION public.web_debug_trigger();
""")
trigger=browse('web_debug_trigger')
target,=trigger['trigger_targets']
assert target['table']=='trigger sandbox' and target['primary_key']==['tenant','ticket id'],target
assert target['operations']==['INSERT','UPDATE','DELETE'],target
assert not next(column for column in target['columns'] if column['name']=='derived')['writable']
key={'tenant':'7','ticket id':'11'}
def trigger_start(operation,columns=None,row_key=None,status=200,**overrides):
    return start(trigger,values=[],status=status,trigger={
        'oid':target['oid'],'expected_definition':target['definition'],'operation':operation,
        'values':columns or {},'key':row_key or {},**overrides})
start(trigger,values=[],status=400)
trigger_start('DELETE',row_key={'ticket id':'11'},status=400)
trigger_start('UPDATE',{'payload':'{}'},key,status=400)
trigger_start('UPDATE',{'label':'x','derived':'12'},key,status=400)
trigger_start('INSERT',{'unknown':'x'},status=400)
trigger_start('DELETE',row_key=key,expected_definition='stale',status=409)
probe="text ' ; DELETE FROM trigger_audit; --"
for operation,columns,row_key in [
    ('INSERT',{'tenant':'7','ticket id':'13','label':probe,'payload':'{"ok":true}'},None),
    ('UPDATE',{'label':None},key),
    ('DELETE',None,key),
]:
    state=wait(trigger_start(operation,columns,row_key),'paused')
    assert value(state,'operation_seen')==operation and state['position']['oid']==trigger['oid'],state
    result=finished(command(state,{'command':'continue'}),'succeeded')
    assert operation in result['outcome']['text'] and 'строк: 1' in result['outcome']['text'],result
    assert sql('SELECT count(*) FROM public.trigger_audit')=='0'
    assert sql('SELECT string_agg(label,\',\' ORDER BY tenant) FROM public."trigger sandbox"')=='original,other'
result=finished(trigger_start('DELETE',row_key={'tenant':'999','ticket id':'11'}),'succeeded')
assert result['pause_id']==0 and 'строк: 0' in result['outcome']['text'],result
state=wait(trigger_start('UPDATE',{'label':'cancelled'},key),'paused')
result=finished(command(state,{'command':'stop'}),'failed')
assert result['outcome']['cancellation_requested']
assert sql('SELECT label FROM public."trigger sandbox" WHERE tenant=7')=='original'
print('HTTP trigger debugging: one-row INSERT/UPDATE/DELETE, composite key, UPDATE OF, NULL, zero rows and rollback passed.')
# Откат после реально выполненного INSERT и отмены.
sql('CREATE TABLE public.web_debug_guard(value integer)')
guard=copy.deepcopy(project);guard['name']='web_debug_guard_fn';guard['blocks'].insert(0,{'id':'insert','type':'sql','sql':'INSERT INTO public.web_debug_guard VALUES (1);'})
install(guard);details=browse(guard['name']);state=wait(start(details,p=guard),'paused')
state=wait(command(state,{'command':'next'}),'paused')
result=finished(command(state,{'command':'stop'}),'failed')
assert result['outcome']['cancellation_requested'];assert sql('SELECT count(*) FROM public.web_debug_guard')=='0'
# При успехе изменения данных тоже откатываются.
state=wait(start(details,p=guard),'paused');finished(command(state,{'command':'continue'}),'succeeded')
assert sql('SELECT count(*) FROM public.web_debug_guard')=='0'
# Ошибка SQL отличается от закрытия отладочного канала.
failure=browse('debug_failure');state=wait(start(failure),'paused')
result=finished(command(state,{'command':'continue'}),'failed')
assert 'P0001' in result['outcome']['text']
# Процедура с INOUT и OUT.
sql("CREATE PROCEDURE public.web_debug_proc(IN input_value integer, INOUT counter integer, OUT result text) LANGUAGE plpgsql AS $$ BEGIN counter := counter + input_value; result := counter::text; END; $$")
details=browse('web_debug_proc');state=wait(start(details,values=['3','2',None]),'paused')
result=finished(command(state,{'command':'continue'}),'succeeded');assert '5' in result['outcome']['text']
# Проверка устаревшего определения не запускает изменившуюся функцию.
sql('ALTER FUNCTION public.web_debug_test(integer) COST 101')
start(selected,status=409)
# Heartbeat продлевает аренду; без него отмена через 15 секунд.
details=browse('debug_smoke');state=wait(start(details),'paused')
post('/api/debug/poll',{'session_id':state['session_id'],'revision':state['revision']})
time.sleep(16)
result=finished(poll(state),'failed');assert result['outcome']['cancellation_requested']
# После завершения слот освобождён, запуск повторяется.
state=wait(start(details),'paused');finished(command(state,{'command':'continue'}),'succeeded')
post('/api/disconnect',{})
print('HTTP debug: real pauses, correct source/block lines, next/continue, breakpoint add/drop, variables, stale commands, rollback on success/stop, SQL failure, OUT procedure, definition conflict, resume, 15s lease and restart passed.')

# Ctrl+C приложения должен дождаться отмены своего активного вызова.
if os.environ.get('PGDEBUG_WEB_TEST_PID'):
    import signal
    identity=post('/api/connect',credentials)['connection_id']
    sql("CREATE FUNCTION public.web_debug_cleanup() RETURNS integer LANGUAGE plpgsql AS $$\nBEGIN\n INSERT INTO public.web_debug_guard VALUES (2);\n PERFORM pg_sleep(60);\n RETURN 1;\nEXCEPTION WHEN query_canceled THEN\n PERFORM pg_sleep(2);\n RAISE;\nEND;\n$$")
    details=browse('web_debug_cleanup');state=wait(start(details,values=[]),'paused')
    state=wait(command(state,{'command':'next'}),'paused')
    state=command(state,{'command':'continue'})
    until=time.monotonic()+5
    while sql("SELECT count(*) FROM pg_stat_activity WHERE query LIKE '%web_debug_cleanup%' AND wait_event='PgSleep'")=='0':
        assert time.monotonic()<until,'Target did not enter pg_sleep'
        time.sleep(.05)
    shutdown_started=time.monotonic()
    os.kill(int(os.environ['PGDEBUG_WEB_TEST_PID']),signal.SIGINT)
    deadline=time.monotonic()+12
    stopped=False
    while time.monotonic()<deadline:
        try:urllib.request.urlopen(base,timeout=1).read()
        except urllib.error.HTTPError:pass # 503 означает закрытие, а не завершённый процесс.
        except (OSError,urllib.error.URLError):stopped=True;break
        time.sleep(.1)
    assert stopped,'Graceful shutdown did not complete'
    assert time.monotonic()-shutdown_started>=1.8,'Server exited before the target cleanup finished'
    assert sql('SELECT count(*) FROM public.web_debug_guard')=='0'
    print(f'Graceful server shutdown waited {time.monotonic()-shutdown_started:.1f}s for cleanup and rolled back the target transaction.')
