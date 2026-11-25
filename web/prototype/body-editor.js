import {mountRecovery} from './recovery.js';
import {validateBodyRecovery} from './working-draft.js';
import {bodyTarget,sameBodyTarget} from './body-target.js';
import {registerHistory,refreshHistory} from './history-ui.js';
import {createCodeEditor} from './vendor/code-editor.js';
import {mountSqlHints} from './sql-hints.js';
import {requestDatabase} from './database-client.js';
const token=document.querySelector('meta[name="pg-debugger-token"]')?.content;
if(token){
  const root=document.getElementById('editor-view');
  root.innerHTML=`<div class="section-head"><div><h1 id="body-title">Редактор тела PL/pgSQL</h1></div><button id="body-debug">Перейти к отладке</button></div><div class="body-toolbar"><button id="body-load">Загрузить выбранный объект</button><button id="body-check">Проверить тело</button><button id="body-reset">Отменить правки</button><button id="body-download">Скачать тело</button></div><p id="body-recovery-state" role="status"></p><section id="body-base-warning" hidden><p>Черновик основан на другом определении или подключении. Сравните сохранённое и текущее определения перед продолжением.</p><button id="body-rebase">Использовать текущее определение как основу</button><details><summary>Сравнить определения</summary><div class="definition-diff"><pre id="body-base-before"></pre><pre id="body-base-current"></pre></div></details></section><p id="body-status" role="status">Выберите функцию или процедуру в каталоге.</p><div class="code-toolbar"><button id="body-find">Найти</button><button id="body-complete">Подсказки</button><label><input id="body-wrap" type="checkbox"> Переносить длинные строки</label></div><div id="body-code"></div><div id="body-hints"></div><details id="body-definition"><summary>Полное определение</summary><pre></pre></details><dialog id="body-preview" aria-labelledby="body-preview-title">
    <h2 id="body-preview-title">Применить изменение?</h2>
    <p id="body-preview-target"></p>
    <div class="definition-diff">
      <section><h3>Сейчас в базе</h3><pre id="body-before"></pre></section>
      <section><h3>После применения</h3><pre id="body-after"></pre></section>
    </div>
    <details><summary>Полные определения</summary><div class="definition-diff"><pre id="body-full-before" aria-label="Сейчас в базе"></pre><pre id="body-full-after" aria-label="После применения"></pre></div></details>
    <p id="body-preview-error" role="alert" hidden></p>
    <div class="body-toolbar"><button id="body-apply" type="button" disabled>Применить в базе</button><form method="dialog"><button autofocus>Закрыть</button></form></div>
  </dialog>`;
  const $=id=>document.getElementById(id);
  const previewDialog=$('body-preview');
  let catalog=null,loaded=null,preview=null,pending=false,busy=false,active=false,revision=0,bodyRecovery=null;
  let hints,comparisonBefore=null,comparisonCurrent=null;
  const editor=createCodeEditor($('body-code'),{nonce:token,label:'Тело функции или процедуры',complete:(source,pos)=>hints?.complete(source,pos),onChange:()=>{invalidate();bodyRecovery?.changed();$('body-status').textContent=dirty()?'Есть неприменённые правки.':'Тело совпадает с загруженным определением.';}});
  hints=mountSqlHints($('body-hints'),()=>loaded?.selected.arguments||[],()=>editor.invalidateCompletions());
  const normalizeLines=text=>text.replace(/\r\n?/g,'\n');
  const dirty=()=>loaded && editor.getValue()!==normalizeLines(loaded.selected.source);
  function bodyForDatabase(){
    const text=editor.getValue(),source=loaded.selected.source;
    if(source.includes('\r\n')&&!/[\r\n]/.test(source.replaceAll('\r\n',''))){
      return text.replaceAll('\n','\r\n');
    }
    if(source.includes('\r')&&!source.includes('\n'))return text.replaceAll('\n','\r');
    return text;
  }
  const same=()=>loaded && catalog?.selected && loaded.connection_id===catalog.connection_id && loaded.database===catalog.database && loaded.selected.oid===catalog.selected.oid && loaded.selected.definition===catalog.selected.definition;
  const allowed=()=>same() && catalog.routines.find(r=>r.oid===catalog.selected.oid)?.debug_allowed;
  function controls(){const locked=pending||busy||active;editor.setReadOnly(pending||active||!loaded);$('body-complete').disabled=pending||active||!loaded;$('body-load').disabled=locked||!catalog?.selected;$('body-check').disabled=locked||!allowed()||!dirty();$('body-apply').disabled=locked||!allowed()||!preview;$('body-reset').disabled=locked||!dirty();$('body-debug').disabled=locked||!catalog?.selected;$('body-download').disabled=!loaded;
    const stale=dirty()&&!same();$('body-base-warning').hidden=!stale;
    $('body-rebase').disabled=locked||!sameBodyTarget(bodyTarget(loaded),bodyTarget(catalog))||same();
    if(stale){
      const before=loaded.selected.definition,current=sameBodyTarget(bodyTarget(loaded),bodyTarget(catalog))?catalog.selected.definition:'Выберите исходный объект в нужной базе и роли.';
      if(comparisonBefore!==before){$('body-base-before').textContent=before;comparisonBefore=before;}
      if(comparisonCurrent!==current){$('body-base-current').textContent=current;comparisonCurrent=current;}
    }
    bodyRecovery?.refresh();refreshHistory();}
  function invalidate(){revision++;preview=null;previewDialog.close();controls();}
  previewDialog.addEventListener('close',()=>{if(!previewDialog.open){preview=null;controls();}});
  document.addEventListener('workspaceview',event=>{
    if(event.detail.view!=='editor'&&previewDialog.open)invalidate();
  });
  function showChanges(before,after){
    const oldLines=normalizeLines(before).split('\n'),newLines=normalizeLines(after).split('\n');
    let start=0,oldEnd=oldLines.length,newEnd=newLines.length;
    while(start<oldEnd&&start<newEnd&&oldLines[start]===newLines[start])start++;
    while(oldEnd>start&&newEnd>start&&oldLines[oldEnd-1]===newLines[newEnd-1]){oldEnd--;newEnd--;}
    const numbered=lines=>lines.map((line,index)=>(start+index+1)+'  '+line).join('\n');
    $('body-before').textContent=numbered(oldLines.slice(start,oldEnd))||'Нет строк';
    $('body-after').textContent=numbered(newLines.slice(start,newEnd))||'Нет строк';
  }
  function load(){
    if(!catalog?.selected)return;
    loaded=JSON.parse(JSON.stringify(catalog));
    const routine=loaded.routines.find(r=>r.oid===loaded.selected.oid);
    $('body-title').textContent=`Тело: ${loaded.schema}.${routine.name}(${routine.signature})`;
    editor.setValue(loaded.selected.source);$('body-definition').querySelector('pre').textContent=loaded.selected.definition;
    invalidate();bodyRecovery?.changed();$('body-status').textContent=allowed()?'':'Для изменения нужен владелец объекта либо администратор PostgreSQL.';
  }
  async function request(action){if(pending)return;pending=true;controls();try{await action();}catch(error){$('body-status').textContent=error.message;if(error.location){editor.goTo(error.location.line,error.location.column);$('body-status').textContent+=`\nСтрока ${error.location.line}, столбец ${error.location.column}.`;}}finally{pending=false;controls();}}
  function input(){return {connection_id:loaded.connection_id,database:loaded.database,routine_oid:loaded.selected.oid,expected_definition:loaded.selected.definition,body:bodyForDatabase()};}
  document.addEventListener('catalogchange',event=>{
    catalog=event.detail;
    if(dirty()){
      invalidate();$('body-status').textContent=same()?'Правки тела сохранены в рабочей области. Перед применением повторите проверку.':'Выбран другой объект, подключение или определение изменилось. Черновик оставлен в редакторе; применение заблокировано. Загрузите выбранный объект, чтобы заменить черновик.';
    }else if(catalog?.selected)load();
    else{invalidate();$('body-status').textContent='Выберите функцию или процедуру. Предыдущее тело оставлено для просмотра.';}
    controls();
  });
  document.addEventListener('databaserequest',event=>{busy=event.detail.busy;controls();});
  document.addEventListener('debugstate',event=>{active=event.detail.active;controls();});
  document.addEventListener('definitionpreviewchange',event=>{if(event.detail.owner!=='body')invalidate();});
  $('body-find').onclick=()=>editor.search();$('body-complete').onclick=()=>editor.complete();$('body-wrap').onchange=()=>editor.setWrapping($('body-wrap').checked);
  $('body-load').onclick=()=>{if(dirty()&&!confirm('Заменить неприменённые правки телом выбранного объекта?'))return;load();};
  $('body-reset').onclick=()=>{if(!loaded||!confirm('Отменить правки тела и вернуть загруженный вариант?'))return;editor.replaceValue(loaded.selected.source);invalidate();bodyRecovery?.changed();$('body-status').textContent='Загруженное тело восстановлено. База не изменялась.';};
  $('body-debug').onclick=()=>{if(dirty()){if(!confirm('В базе остаётся прежнее тело. Перейти к его отладке без применения правок?'))return;}document.getElementById('view-debug').click();};
  $('body-check').onclick=()=>request(async()=>{
    const payload=input();invalidate();const checkRevision=revision;
    const result=await requestDatabase('/api/body/prepare',payload);
    if(revision!==checkRevision||!same()||root.hidden){$('body-status').textContent='Объект или черновик изменился. Повторите проверку.';return;}
    preview={...result,input:payload};
    const routine=loaded.routines.find(entry=>entry.oid===loaded.selected.oid);
    $('body-preview-target').textContent=loaded.database+' · '+loaded.schema+'.'+routine.name+'('+routine.signature+')';
    showChanges(loaded.selected.source,payload.body);
    $('body-full-before').textContent=result.before;
    $('body-full-after').textContent=result.after;
    $('body-preview-error').hidden=true;
    previewDialog.querySelector('details').open=false;
    document.dispatchEvent(new CustomEvent('definitionpreviewchange',{detail:{owner:'body'}}));
    previewDialog.showModal();
  });
  $('body-apply').onclick=()=>request(async()=>{
    if(!preview||!previewDialog.open)return;
    const check=preview;preview=null;controls();
    let result;
    try{result=await requestDatabase('/api/body/apply',{...check.input,preview_id:check.preview_id});}
    catch(error){
      $('body-preview-error').textContent=error.message;
      $('body-preview-error').hidden=false;
      $('body-status').textContent=error.message;
      return;
    }
    previewDialog.close();
    loaded.selected.source=check.input.body;loaded.selected.definition=check.after;bodyRecovery?.changed();bodyRecovery?.flush();
    $('body-definition').querySelector('pre').textContent=check.after;
    $('body-status').textContent=result.message+' Обновляем выбранный объект…';
    document.dispatchEvent(new CustomEvent('catalogrefresh',{detail:{routine_oid:loaded.selected.oid,onSuccess:()=>{document.getElementById('view-editor').click();$('body-status').textContent=result.message+' Можно перейти к отладке.';},onError:()=>{$('body-status').textContent=result.message+' Каталог не обновлён. Обновите его перед следующей проверкой.';}}}));
  });
  registerHistory('body',{undo:()=>editor.undo(),redo:()=>editor.redo(),available:()=>({undo:!pending&&!active&&editor.canUndo(),redo:!pending&&!active&&editor.canRedo()})});
  bodyRecovery=mountRecovery({kind:'body',label:'тело',validate:validateBodyRecovery,
    isDirty:()=>!!dirty(),snapshot:()=>({target:bodyTarget(loaded),definition:loaded.selected.definition,body:editor.getValue()}),
    canRestore:value=>!pending&&!active&&sameBodyTarget(value.target,bodyTarget(catalog)),
    onStatus:text=>$('body-recovery-state').textContent=text,
    onRestore:value=>{
      if(dirty()&&!confirm('Заменить текущие правки тела восстановительной копией?'))return false;
      loaded=JSON.parse(JSON.stringify(catalog));
      if(value.definition!==catalog.selected.definition&&value.body!==catalog.selected.source)loaded.selected.definition=value.definition;
      editor.replaceValue(value.body);invalidate();$('start-constructor').click();$('view-editor').click();
      $('body-status').textContent=same()?'Тело восстановлено. Перед применением выполните новую проверку.':'Определение в базе изменилось. Восстановленный текст оставлен; сравните определения и выберите текущую основу явно.';
      return true;
    }
  });
  $('body-download').onclick=()=>{
    if(!loaded)return;const name=bodyTarget(loaded)?.name.replace(/[^a-z_0-9-]/ig,'_')||'routine';
    const url=URL.createObjectURL(new Blob([editor.getValue()],{type:'text/plain;charset=utf-8'}));
    const link=document.createElement('a');link.href=url;link.download=name+'.plpgsql.txt';link.click();setTimeout(()=>URL.revokeObjectURL(url),1000);
    $('body-status').textContent='Тело выгружено в текстовый файл. Это не полный CREATE; определение в базе не менялось.';
  };
  $('body-rebase').onclick=()=>{
    if(!sameBodyTarget(bodyTarget(loaded),bodyTarget(catalog))||pending||active)return;
    if(!confirm('Использовать текущее определение выбранного объекта как основу? Текст черновика останется. Проверка и применение потребуются заново.'))return;
    loaded=JSON.parse(JSON.stringify(catalog));$('body-definition').querySelector('pre').textContent=loaded.selected.definition;invalidate();bodyRecovery.changed();
    $('body-status').textContent='Текущая основа принята. Проверьте восстановленное тело перед применением.';
  };
  controls();
}
