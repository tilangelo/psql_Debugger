import {metadataContext,readMetadata,clearMetadataCache} from './metadata-client.js';
import {completionOptions} from './sql-completions.js';
import {columnHints,tableCaption} from './model-hints.js';

export function mountSqlHints(root,parameters,onChange=()=>{},{variables,initialModel=null,autoLoad=true}={}){
  const section=document.createElement('details');section.className='sql-hints';
  section.innerHTML='<summary>Таблицы и поля</summary><div class="sql-hint-controls"><label>Схема<select data-schema></select></label><label>Таблица<select data-table></select></label><button type="button">Обновить</button></div><p role="status" hidden></p><ul class="sql-hint-columns" aria-label="Поля таблицы" hidden></ul>';
  root.append(section);const schema=section.querySelector('[data-schema]'),table=section.querySelector('[data-table]'),refresh=section.querySelector('button'),message=section.querySelector('p'),columns=section.querySelector('ul');
  let context=null,state=null,model=initialModel,modelKey=JSON.stringify(initialModel),busy=false,active=false,disposed=false;const tables=new Map(),listeners=[];
  function listen(name,handler){document.addEventListener(name,handler);listeners.push([name,handler]);}
  const alive=target=>!disposed&&metadataContext===target;
  function controls(){for(const element of [schema,table,refresh])element.disabled=busy||active||!metadataContext;}
  function options(select,rows,first){select.replaceChildren();if(first){const option=document.createElement('option');option.value='';option.textContent=first;select.append(option);}for(const [value,caption]of rows){const option=document.createElement('option');option.value=value;option.textContent=caption;select.append(option);}}
  function showMessage(text=''){message.textContent=text;message.hidden=!text;}
  function renderColumns(){
    const relation=state?.relations.find(r=>String(r.oid)===table.value);
    const entry=relation&&tables.get(relation.schema+'\0'+relation.name);
    columns.replaceChildren();
    columns.hidden=!entry?.details.columns.length;
    for(const column of entry?.details.columns||[]){
      const item=document.createElement('li'),name=document.createElement('code'),type=document.createElement('span');
      name.textContent=column.name;
      const hint=columnHints(model,relation,column);
      type.textContent=[column.data_type,hint?.label,hint?.description||column.comment].filter(Boolean).join(' · ');
      item.append(name,type);columns.append(item);
    }
  }
  function render(){options(schema,(state?.schemas||[]).map(s=>[s,s]));schema.value=state?.schema||'';options(table,(state?.relations||[]).map(r=>[String(r.oid),tableCaption(model,r)]),'Выберите таблицу');renderColumns();}
  async function loadSchema(force=false){
    if(busy||active||!metadataContext)return;const target=metadataContext;busy=true;controls();showMessage('Загрузка схемы…');
    try{
      if(force){clearMetadataCache();tables.clear();}
      const result=await readMetadata({...target,schema:schema.value||target.schema||null,relation_oid:null},()=>alive(target));
      if(!alive(target))return;context=target;state=result;render();onChange();
      showMessage();
    }catch(error){if(alive(target)){state=null;tables.clear();render();showMessage(error.message);onChange();}}
    finally{busy=false;controls();}
  }
  async function loadTable(){
    renderColumns();showMessage();
    if(busy||active||!table.value||!state||!metadataContext)return;const target=metadataContext,relation=state.relations.find(r=>String(r.oid)===table.value);if(!relation)return;
    busy=true;controls();showMessage('Загрузка полей…');
    try{
      const result=await readMetadata({...target,schema:relation.schema,relation_oid:relation.oid},()=>alive(target));
      if(!alive(target))return;
      if(!result.selected)throw Error('Таблица недоступна. Обновите структуру.');
      if(tables.size>=20)tables.delete(tables.keys().next().value);
      tables.set(relation.schema+'\0'+relation.name,{relation,details:result.selected});onChange();
      renderColumns();showMessage(columns.hidden?'Нет доступных полей.':'');
    }catch(error){if(alive(target)){tables.delete(relation.schema+'\0'+relation.name);renderColumns();showMessage(error.message);onChange();}}
    finally{busy=false;controls();}
  }
  schema.onchange=()=>loadSchema();table.onchange=loadTable;refresh.onclick=()=>loadSchema(true);
  section.addEventListener('toggle',()=>{if(section.open&&context!==metadataContext)loadSchema();});
  listen('workspaceview',event=>{if(autoLoad&&event.detail.view==='editor'&&context!==metadataContext)loadSchema();});
  listen('metadata-context-change',()=>{
    if(context!==metadataContext){context=null;state=null;tables.clear();render();showMessage(metadataContext?'':'Подключитесь к PostgreSQL.');onChange();controls();}
  });
  listen('constructorchange',event=>{const next=JSON.stringify(event.detail.model);if(next===modelKey)return;modelKey=next;model=event.detail.model;const previous=table.value;render();table.value=previous;renderColumns();onChange();});
  listen('debugstate',event=>{active=event.detail.active;controls();});
  showMessage(metadataContext?'':'Подключитесь к PostgreSQL.');controls();
  return {
    complete(source,pos){return completionOptions({source,pos,parameters:parameters(),variables:variables?.(),model,schemas:context===metadataContext?(state?.schemas||[]):[],relations:context===metadataContext?(state?.relations||[]):[],tables:context===metadataContext?[...tables.values()]:[]});},
    destroy(){disposed=true;for(const [name,handler]of listeners)document.removeEventListener(name,handler);tables.clear();section.remove();},
  };
}
