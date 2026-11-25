import {createRecoveryStore,RecoveryConflict} from './recovery-store.js';
// Автокопия не заменяет явное сохранение и никогда сама не применяется к базе.
const controllers=[];
const storage={getItem:key=>localStorage.getItem(key),setItem:(key,value)=>localStorage.setItem(key,value),removeItem:key=>localStorage.removeItem(key)};
export function mountRecovery({kind,label,validate,isDirty,snapshot,onRestore,canRestore=()=>true,onStatus=()=>{}}){
  const key='pgdebug-recovery-'+kind+'-v1',writer=crypto.randomUUID();
  const store=createRecoveryStore(storage,key,kind,validate,{writer});
  const section=document.createElement('div');section.className='recovery-notice';section.dataset.recovery=kind;section.hidden=true;
  const message=document.createElement('p'),restore=document.createElement('button'),discard=document.createElement('button'),retry=document.createElement('button');
  restore.textContent='Восстановить '+label;discard.textContent='Удалить автокопию';retry.textContent='Повторить';section.append(message,restore,discard,retry);document.getElementById('recovery-notices').append(section);
  let pending=null,error=null,readError=false,timer=null,saved=null,owned=false,lastStatus='';
  function status(text){lastStatus=text;onStatus(text);}
  function render(){
    section.hidden=!pending&&!error;
    if(pending){const target=pending.value.target;message.textContent=`Найдена автокопия: ${label}, ${new Date(pending.at).toLocaleString()}.`+(target?` ${target.user}@${target.host}:${target.port}, база ${target.database}, ${target.schema}.${target.name}(${target.signature}). Подключитесь и выберите этот объект для восстановления.`:` ${pending.value.project.schema}.${pending.value.project.name||'(имя не задано)'}. Явно сохранённый проект не заменён.`);}
    else message.textContent=error||'';
    restore.hidden=!pending;restore.disabled=!pending||!canRestore(pending.value);discard.disabled=false;
    retry.hidden=!error;
  }
  function read(){try{pending=store.read();error=null;readError=false;}catch(e){pending=null;error=e.message;readError=true;}render();}
  read();
  function flush(){
    clearTimeout(timer);timer=null;
    if(!isDirty()){
      if(pending||readError)return true;
      try{if(owned)store.clear();owned=false;saved=null;error=null;render();status('');return true;}
      catch(e){if(e instanceof RecoveryConflict)read();else error=e.message;status(e.message);render();return true;}
    }
    if(pending||error){if(isDirty())status('Текущая правка не записана: сначала разберите найденную автокопию.');return false;}
    try{
      const value=validate(snapshot()),json=JSON.stringify(value);
      if(json!==saved){store.write(value);saved=json;owned=true;}
      status('Черновик сохранён в браузере');return true;
    }catch(e){if(e instanceof RecoveryConflict){read();status(e.message);}else{error=e.message;readError=false;status(error);render();}return false;}
  }
  function changed(){
    clearTimeout(timer);render();
    if(!isDirty()){timer=setTimeout(flush,0);return;}
    status(pending||error?'Текущая правка не записана: найденная автокопия оставлена без изменений.':'Сохраняем черновик…');
    if(!pending&&!error)timer=setTimeout(flush,600);
  }
  restore.onclick=async()=>{
    if(!pending||!canRestore(pending.value))return;
    try{
      if(!store.unchanged()){read();status('Автокопия обновилась в другой вкладке. Проверьте её и повторите восстановление.');return;}
      const accepted=await onRestore(pending.value);if(accepted===false)return;
      if(!store.unchanged()){read();status('Автокопия изменена в другой вкладке. Восстановленный текст оставлен для просмотра.');return;}
      pending=null;error=null;owned=true;saved=null;render();changed();flush();
    }catch(e){error=e.message;status(error);render();}
  };
  discard.onclick=()=>{
    if(!confirm('Удалить эту восстановительную автокопию? Текст в рабочей области и явно сохранённый проект останутся.'))return;
    try{store.clear();pending=null;error=null;owned=false;saved=null;render();changed();}
    catch(e){read();status(e.message);}
  };
  retry.onclick=()=>{if(readError){read();return;}error=null;saved=null;render();flush();};
  window.addEventListener('storage',event=>{if(event.key===key){clearTimeout(timer);saved=null;owned=false;read();status('Автокопия изменена в другой вкладке. Текущая рабочая область не заменена.');}});
  const controller={changed,flush,refresh:render,
    safeToLeave(){try{if(!isDirty()){flush();return true;}return flush()&&store.unchanged();}catch(e){status(e.message);return false;}},
    lastStatus:()=>lastStatus,
  };
  controllers.push(controller);return controller;
}
function flushAll(){for(const controller of controllers)controller.flush();}
window.addEventListener('pagehide',flushAll);
document.addEventListener('visibilitychange',()=>{if(document.hidden)flushAll();});
window.addEventListener('beforeunload',event=>{let safe=true;for(const controller of controllers)if(!controller.safeToLeave())safe=false;if(!safe){event.preventDefault();event.returnValue='';}});
