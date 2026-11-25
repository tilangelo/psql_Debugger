import './debug.js';
import {readiness,canEnableDebugger} from './readiness.js';
import {requestDatabase} from './database-client.js';
import {bodyTarget,sameBodyTarget} from './body-target.js';
import {workPresets} from './work-presets.js';
// Выбор объекта в каталоге не меняет черновик конструктора.
const token=document.querySelector('meta[name="pg-debugger-token"]')?.content;
if(token){
  const panel=document.getElementById('database-panel'), toggle=document.getElementById('database-toggle');
  toggle.hidden=false;
  toggle.insertAdjacentHTML('afterend', '<details class="compact-menu align-end" id="connection-menu" hidden><summary id="connection-menu-label">Подключение</summary><div class="compact-menu-items"><button type="button" id="connection-change">Настройки подключения</button><button type="button" id="db-disconnect">Отключиться</button></div></details>');
  const dialog=document.getElementById('connection-dialog');
  dialog.innerHTML=`<div class="catalog-heading"><h2 id="connection-title">Подключение к PostgreSQL</h2><button type="button" id="db-close">Закрыть</button></div>
  <p id="db-intent"></p><form id="db-form" class="db-form"><label>Адрес<input id="db-host" value="127.0.0.1" required></label><label>Порт PostgreSQL<input id="db-port" value="5432" type="number" min="1" max="65535" required inputmode="numeric"></label><label>База<input id="db-name" value="postgres" required></label><label>Пользователь<input id="db-user" value="postgres" autocomplete="off" required></label><label>Пароль<input id="db-password" type="password" autocomplete="off"></label><button type="submit">Подключиться</button></form>
  <p id="connection-status" role="status"></p>`;
  panel.innerHTML=`<h1>Каталог PostgreSQL</h1><p id="db-status" role="status"></p><div id="db-browser" hidden><div class="db-controls"><label>База данных<select id="db-database"></select></label><label>Схема<select id="db-schema"></select></label><label class="db-search">Поиск<input id="db-search" placeholder="Имя или сигнатура"></label><button id="refresh-catalog">Обновить</button></div><div class="apply-actions"><button id="prepare-definition">Проверить в базе</button><button id="apply-definition" disabled>Применить в базе</button></div><dialog id="definition-dialog" aria-labelledby="preview-title"><form method="dialog"><button class="dialog-close">Закрыть</button></form><section id="definition-preview" hidden><h2 id="preview-title">Изменение определения</h2><p id="preview-message"></p><div class="definition-diff"><div><h3>Сейчас в базе</h3><pre id="definition-before"></pre></div><div><h3>После применения</h3><pre id="definition-after"></pre></div></div></section></dialog><div id="db-content" class="db-content"><div><div id="db-routines"></div><button type="button" id="db-create">Создать в конструкторе</button></div><section id="db-routine-detail" hidden><h2 id="db-definition-title">Исходный код</h2><p id="db-routine-status" role="status" tabindex="-1" hidden></p><p id="db-arguments"></p><pre id="db-definition" tabindex="0">Выберите функцию или процедуру.</pre></section></div></div>`;
  const $=id=>document.getElementById(id);let intent='connect',current=null,busy=false,draft=null,preview=null,revision=0,debugActive=false,databaseBusy=false;
  let profiles=[],recent=[],needsSetup=false;
  $('db-definition-title').insertAdjacentHTML('afterend',`<div id="db-routine-actions" class="catalog-routine-actions" hidden>
    <button id="db-routine-debug" type="button">Отладить</button>
    <button id="db-routine-edit" type="button">Редактировать</button></div>`);
  for(const [action,view,title] of [['debug','debug','debug-title'],['edit','editor','body-title']]){
    $('db-routine-'+action).onclick=()=>{
      if(!current?.selected)return;
      $('view-'+view).click();
      const heading=$(title);heading.tabIndex=-1;heading.focus();
    };
  }
  $('db-create').onclick=()=>{
    if(current&&!needsSetup)document.dispatchEvent(new CustomEvent('constructornew',{detail:{schema:current.schema||'public'}}));
  };
  $('db-form').insertAdjacentHTML('beforebegin',`<div id="db-profile-bar" class="preset-row" hidden>
    <label>Профиль подключения<select id="db-profile"></select></label>
    <button type="button" id="db-profile-remove">Удалить профиль</button></div>`);
  $('db-password').parentElement.insertAdjacentHTML('afterend',`<label id="db-profile-save-label" class="preset-choice">
    <input type="checkbox" id="db-profile-save"> Сохранить адрес и пользователя без пароля</label>`);
  $('db-browser').insertAdjacentHTML('afterbegin',`<details id="db-recent-panel" hidden><summary>Недавние объекты</summary>
    <div class="preset-row"><label>Функция или процедура<select id="db-recent"></select></label>
    <button id="db-recent-open" type="button">Открыть</button><button id="db-recent-clear" type="button">Очистить</button></div>
    <p id="db-recent-status" role="status"></p></details>`);
  const start=$('start-screen');
  const isolatedDemo=!!document.querySelector('meta[name="pg-debugger-isolated-demo"]');
  if(isolatedDemo)start.insertAdjacentHTML('beforeend','<p id="isolated-demo-status" role="status" hidden></p>');
  const setup=document.createElement('section');
  setup.id='db-readiness';
  setup.hidden=true;
  setup.setAttribute('aria-labelledby','db-setup-title');
  setup.innerHTML='<h2 id="db-setup-title" tabindex="-1"></h2><p id="db-setup-action"></p><p id="db-setup-scope" hidden></p><div class="catalog-routine-actions"><button id="db-enable-extension" type="button" hidden>Включить в этой базе</button><button id="db-refresh-setup" type="button">Обновить проверку</button></div><p id="db-setup-result" role="status" hidden></p><details id="db-setup-checks"><summary>Сведения о проверке</summary><dl></dl></details><details id="db-setup-detail" hidden><summary>Подробности PostgreSQL</summary><pre></pre></details>';
  $('db-content').before(setup);
  function showSetupResult(message=''){
    const [short,...detail]=message.split('\\n');
    const result=$('db-setup-result'),more=$('db-setup-detail');
    result.textContent=short;result.hidden=!short;
    more.querySelector('pre').textContent=detail.join('\\n');
    more.hidden=!detail.length;more.open=false;
  }
  function setupSelection(){return {connection_id:current.connection_id,database:current.database,schema:current.schema,routine_oid:current.selected?.oid??null};}
  function runSetup(install){
    if(!current||busy||databaseBusy||debugActive)return;
    request(async()=>{
      showSetupResult(install?'Включаем расширение…':'Проверяем компоненты…');
      try{
        const result=await api(install?'/api/debug/setup':'/api/catalog',setupSelection());
        render(result);
        const message=install?'Расширение pldbgapi включено в выбранной базе.':'Проверка обновлена.';
        if(needsSetup)showSetupResult(message);
        else status(message);
      }catch(error){showSetupResult(error instanceof TypeError?'Не удалось связаться с приложением. Проверьте, что оно запущено, и обновите страницу.':error.message);}
    },'');
  }
  $('db-enable-extension').onclick=()=>{if(canEnableDebugger(current))runSetup(true);};
  $('db-refresh-setup').onclick=()=>runSetup(false);
  const applyActions=panel.querySelector('.apply-actions'),definition=$('definition-dialog');
  $('constructor-db-actions').append(applyActions);
  $('definition-preview').append($('apply-definition'));
  document.body.append(definition);
  applyActions.hidden=true;
  function welcome(){panel.hidden=true;start.hidden=false;document.body.classList.add('onboarding');$(isolatedDemo?'start-demo':'start-connect').focus();}
  function workbench(){start.hidden=true;document.body.classList.remove('onboarding');}
  function openConnection(next='connect'){
    intent=next;
    $('db-intent').textContent=next==='demo'?'Будет создана отдельная тестовая база. Нужна роль администратора.':'';
    $('connection-status').textContent='';
    $('db-password').value='';
    $('db-profile-save').checked=false;
    $('db-profile-save-label').hidden=next==='demo';
    renderProfiles();
    $('db-form').querySelector('button[type=submit]').textContent=next==='demo'?'Создать тестовую базу':'Подключиться';
    if(!dialog.open)dialog.showModal();
  }
  $('start-connect').onclick=()=>openConnection();
  $('start-demo').onclick=()=>isolatedDemo
    ?request(async()=>showDemo(await api('/api/demo/open-isolated',{})),'Открываем временную тестовую базу…')
    :openConnection('demo');
  $('start-constructor').onclick=()=>{workbench();$('view-constructor').click();};
  document.addEventListener('debugstate',event=>{if(event.detail.active)workbench();});
  welcome();
  function status(message){
    const id=dialog.open?'connection-status':isolatedDemo&&!start.hidden?'isolated-demo-status':panel.hidden?'constructor-db-status':'db-status';
    $(id).textContent=message;
    if(id==='isolated-demo-status')$(id).hidden=!message;
  }
  function renderReadiness(result,checks){
    const problem=checks.find(check=>check.state==='blocked');
    $('db-setup-title').textContent=problem?.label||'Готовность не подтверждена';
    $('db-setup-action').textContent=problem?.message||'Обновите проверку подключения.';
    const list=setup.querySelector('dl');
    list.replaceChildren();
    for(const check of checks.filter(check=>check!==problem)){
      const title=document.createElement('dt'),description=document.createElement('dd');
      title.textContent=check.label;
      title.dataset.state=check.state;
      description.textContent=check.message;
      list.append(title,description);
    }
    const enable=canEnableDebugger(result);
    $('db-enable-extension').hidden=!enable;
    $('db-setup-scope').hidden=!enable;
    $('db-setup-scope').textContent=enable?'Расширение будет создано в базе «'+result.database+'».':'';
  }
  function focusRoutineAction(){
    ($('db-routine-status').hidden?$('db-routine-debug'):$('db-routine-status')).focus();
  }
  function controls(){const blocked=busy||debugActive||databaseBusy;document.querySelectorAll('#database-panel input,#database-panel select,#database-panel button,#connection-dialog input,#connection-dialog select,#connection-dialog button,#constructor-db-actions button,#definition-dialog #apply-definition').forEach(e=>e.disabled=blocked);$('apply-definition').disabled=blocked||!preview||!current;$('prepare-definition').disabled=blocked||!draft||!current;
    $('db-routine-debug').disabled=blocked||!$('db-routine-status').hidden;
    $('connection-change').disabled=$('db-disconnect').disabled=blocked;
    $('db-enable-extension').disabled=blocked||!canEnableDebugger(current);
    $('db-refresh-setup').disabled=blocked||!current;
    $('start-demo').disabled=$('start-connect').disabled=$('start-constructor').disabled=blocked;
    $('db-profile-remove').disabled=blocked||!$('db-profile').value;
    $('db-recent-open').disabled=blocked||!$('db-recent').value;
    $('db-recent-clear').disabled=blocked||!recent.length;
  }
  document.addEventListener('databaserequest',event=>{databaseBusy=event.detail.busy;controls();});
  document.addEventListener('debugstate',event=>{debugActive=event.detail.active;if(debugActive)invalidate();else controls();});
  function invalidate(){preview=null;definition.close();$('definition-preview').hidden=true;controls();}
  document.addEventListener('constructorchange',event=>{draft=event.detail;revision++;invalidate();});
  $('db-close').onclick=()=>dialog.close();
  toggle.onclick=()=>openConnection();
  $('connection-change').onclick=()=>{$('connection-menu').open=false;openConnection();};
  const api=requestDatabase;
  async function request(action,message='Загружаем…'){
    if(busy)return;
    busy=true;controls();status(message);
    try{await action();}
    catch(error){status(error.message);if(current){$('db-database').value=current.database;$('db-schema').value=current.schema||'';}}
    finally{busy=false;controls();}
  }
  function options(id,values,selected){$(id).replaceChildren();for(const value of values){const option=document.createElement('option');option.value=value;option.textContent=value;option.selected=value===selected;$(id).append(option);}}
  function presetOptions(id,entries,caption,placeholder){
    $(id).replaceChildren(new Option(placeholder,''));
    entries.forEach((entry,index)=>$(id).add(new Option(caption(entry),String(index))));
  }
  function endpoint(result){return {...result.endpoint,database:result.database,user:result.current_user};}
  function renderProfiles(){
    profiles=[];
    try{if(intent==='connect')profiles=workPresets.profiles();}
    catch(error){$('connection-status').textContent=error.message;}
    presetOptions('db-profile',profiles,p=>`${p.user}@${p.host}:${p.port} / ${p.database}`,'Выберите профиль');
    $('db-profile-bar').hidden=!profiles.length;
    controls();
  }
  $('db-profile').onchange=()=>{
    const profile=profiles[$('db-profile').value];
    if(profile){
      for(const [field,key] of [['db-host','host'],['db-port','port'],['db-name','database'],['db-user','user']])$(field).value=profile[key];
      $('db-password').value='';
      $('db-password').focus();
    }
    controls();
  };
  $('db-profile-remove').onclick=()=>{
    const profile=profiles[$('db-profile').value];
    if(!profile)return;
    try{workPresets.removeProfile(profile);renderProfiles();status('Профиль удалён.');}
    catch(error){status(error.message);}
  };
  function renderRecent(){
    recent=[];$('db-recent-status').textContent='';
    if(current&&!needsSetup){
      try{
        const target=bodyTarget(current);
        if(target)workPresets.remember(target);
        recent=workPresets.recent(endpoint(current));
      }catch(error){$('db-recent-status').textContent=error.message;}
    }
    presetOptions('db-recent',recent,r=>`${r.database} · ${r.schema}.${r.name}(${r.signature})`,'Выберите объект');
    $('db-recent-panel').hidden=!recent.length&&!$('db-recent-status').textContent;
    controls();
  }
  $('db-recent').onchange=controls;
  $('db-recent-clear').onclick=()=>{
    try{
      workPresets.clearRecent(endpoint(current));
      recent=[];
      $('db-recent-panel').hidden=true;
      controls();
    }catch(error){$('db-recent-status').textContent=error.message;}
  };
  $('db-recent-open').onclick=()=>request(async()=>{
    const target=recent[$('db-recent').value];
    if(!target)return;
    $('db-recent-status').textContent='';
    const payload={connection_id:current.connection_id,database:target.database,schema:target.schema,routine_oid:null};
    const listing=await api('/api/catalog',payload);
    const match=listing.routines.find(r=>listing.schema===target.schema&&r.name===target.name&&r.signature===target.signature&&r.kind===target.kind);
    $('db-status').textContent='';
    if(!match){$('db-recent-status').textContent='Объект больше не найден в каталоге. Выберите актуальную сигнатуру.';return;}
    const result=await api('/api/catalog',{...payload,routine_oid:match.oid});
    if(!sameBodyTarget(bodyTarget(result),target)){$('db-recent-status').textContent='Объект изменился. Обновите каталог.';return;}
    render(result);
    requestAnimationFrame(focusRoutineAction);
  });
  function routines(){
    const list=$('db-routines');list.replaceChildren();
    const query=$('db-search').value.toLocaleLowerCase();
    const rows=current.routines.filter(r=>(r.name+' '+r.signature).toLocaleLowerCase().includes(query));
    if(!rows.length){list.textContent='Доступных PL/pgSQL-функций и процедур не найдено.';return;}
    for(const routine of rows){
      const button=document.createElement('button');
      button.type='button';
      button.textContent=`${routine.kind}: ${routine.name}(${routine.signature})`;
      button.setAttribute('aria-pressed',String(routine.oid===current.selected?.oid));
      button.onclick=()=>{
        if(routine.oid===current.selected?.oid){focusRoutineAction();return;}
        request(async()=>{
          await browse({routine_oid:routine.oid});
          if(current.selected?.oid===routine.oid)requestAnimationFrame(focusRoutineAction);
        });
      };
      list.append(button);
    }
  }
  function render(result){
    const checks=readiness(result,{includeObject:false});
    const wasBlocked=needsSetup;
    needsSetup=result.debugger_code==='unknown'||checks.some(check=>check.state==='blocked');
    if(current?.database!==result.database||current?.connection_id!==result.connection_id)$('db-search').value='';
    current=result;
    workbench();invalidate();
    $('view-catalog').hidden=false;
    toggle.hidden=true;
    $('connection-menu').hidden=false;
    const connectionLabel=$('connection-menu-label');
    connectionLabel.textContent=result.database;
    connectionLabel.title='Подключение к '+result.database;
    connectionLabel.setAttribute('aria-label',connectionLabel.title);
    $('db-browser').hidden=false;
    $('db-content').hidden=needsSetup;
    setup.hidden=!needsSetup;
    applyActions.hidden=needsSetup;
    for(const id of ['db-schema','db-search'])$(id).parentElement.hidden=needsSetup;
    $('refresh-catalog').hidden=needsSetup;
    options('db-database',result.databases,result.database);
    options('db-schema',result.schemas,result.schema);
    $('db-status').textContent='';
    showSetupResult();
    const selected=result.selected,routine=result.routines.find(r=>r.oid===selected?.oid);
    const problems=routine?readiness(result).slice(checks.length).filter(check=>check.state==='blocked'):[];
    $('db-routine-status').textContent=problems.map(check=>check.message).join(' ');
    $('db-routine-status').hidden=!problems.length;
    $('db-routine-actions').hidden=!routine;
    $('db-routine-detail').hidden=!routine;
    $('db-definition-title').textContent=routine?routine.kind+': '+routine.name+'('+routine.signature+')':'Исходный код';
    $('db-definition').textContent=selected?.definition||'Выберите функцию или процедуру.';
    $('db-arguments').textContent=selected?selected.arguments.map(a=>a.mode+' '+(a.name||'без имени')+': '+a.type).join(', '):'';
    $('db-routines').replaceChildren();
    if(needsSetup){
      renderReadiness(result,checks);
      requestAnimationFrame(()=>$('db-setup-title').focus());
    }else{
      routines();
      if(wasBlocked)requestAnimationFrame(()=>($('db-routines').querySelector('button')||$('db-search')).focus());
    }
    document.dispatchEvent(new CustomEvent('catalogchange',{detail:needsSetup?null:result}));
    renderRecent();
  }
  async function browse(extra={}){const result=await api('/api/catalog',{connection_id:current.connection_id,database:$('db-database').value,schema:$('db-schema').value||null,routine_oid:null,...extra});render(result);}
  function showDemo(result){
    if(isolatedDemo){$('isolated-demo-status').textContent='';$('isolated-demo-status').hidden=true;}
    render(result);$('view-debug').click();
    setTimeout(()=>document.dispatchEvent(new CustomEvent('demoready',{detail:result.demo})),0);
  }
  $('db-form').onsubmit=event=>{event.preventDefault();request(async()=>{
    const credentials={host:$('db-host').value,port:$('db-port').value,database:$('db-name').value,user:$('db-user').value,password:$('db-password').value};
    try{
      const connected=await api('/api/connect',credentials);
      let profileMessage='';
      if(intent==='demo'){
        status('Создаём тестовую базу…');
        const created=await api('/api/demo',{connection_id:connected.connection_id,database:connected.database,confirmed:true});
        showDemo(created);$('db-name').value=created.database;
      }else{
        if($('db-profile-save').checked){
          try{workPresets.saveProfile(endpoint(connected));profileMessage='Профиль сохранён без пароля.';}
          catch(error){profileMessage='Подключение установлено. Профиль не сохранён. '+error.message;}
        }
        render(connected);$('view-catalog').click();
      }
      dialog.close();
      if(profileMessage)status(profileMessage);
    }finally{$('db-password').value='';}
  });};
  $('db-database').onchange=()=>request(()=>browse({schema:null}));
  $('db-schema').onchange=()=>request(()=>browse());
  $('db-search').oninput=routines;
  $('db-disconnect').onclick=()=>request(async()=>{await api('/api/disconnect',{});current=null;needsSetup=false;setup.hidden=true;applyActions.hidden=true;$('view-catalog').hidden=true;toggle.hidden=false;$('connection-menu').hidden=true;$('connection-menu').open=false;document.dispatchEvent(new CustomEvent('catalogchange',{detail:null}));invalidate();$('db-browser').hidden=true;$('db-definition').textContent='';$('db-routines').replaceChildren();$('db-status').textContent='Подключение удалено из памяти сервера.';$('db-readiness').querySelector('dl').replaceChildren();welcome();});
  document.addEventListener('definitionpreviewchange',event=>{if(event.detail.owner!=='constructor')invalidate();});
  document.addEventListener('catalogrefresh',event=>request(async()=>{try{await browse({routine_oid:event.detail.routine_oid});event.detail.onSuccess?.();}catch(error){event.detail.onError?.(error);throw error;}}));
  $('refresh-catalog').onclick=()=>request(()=>browse());
  $('prepare-definition').onclick=()=>request(async()=>{
    const version=revision,project=JSON.parse(JSON.stringify(draft));invalidate();
    const result=await api('/api/prepare',{connection_id:current.connection_id,database:current.database,project});
    if(version!==revision){$('db-status').textContent='Проект изменился во время проверки. Повторите проверку.';return;}
    document.dispatchEvent(new CustomEvent('definitionpreviewchange',{detail:{owner:'constructor'}}));preview=result;$('definition-preview').hidden=false;definition.showModal();
    $('preview-title').textContent=result.action==='create'?'Создание нового объекта':'Замена существующего определения';
    $('preview-message').textContent=`База: ${result.database}. Сигнатура: ${result.signature}. ${result.message}`;
    $('definition-before').textContent=result.before||'Объект с этой сигнатурой отсутствует. Другие перегрузки сохранятся.';
    $('definition-after').textContent=result.after;
    status('Определение проверено.');
  },'Проверяем определение в PostgreSQL…');
  $('apply-definition').onclick=()=>{
    if(!preview)return;
    const pending=preview;preview=null;controls();
    request(async()=>{
      const result=await api('/api/apply',{connection_id:current.connection_id,database:current.database,preview_id:pending.preview_id,project:draft});
      definition.close();$('definition-preview').hidden=true;
      try{await browse();status(result.message);}catch(error){status(result.message+' Каталог не обновлён: '+error.message);}
    },'Сохраняем определение в базе…');
  };
  controls();

}

