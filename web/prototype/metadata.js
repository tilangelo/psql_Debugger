import {buildSelect, columnAllowed, selectModes} from './select-builder.js';
import {metadataContext as context,readMetadata,clearMetadataCache} from './metadata-client.js';
import {tableCaption,columnCaption,tableHints,columnHints} from './model-hints.js';
import {mountHintsEditor} from './hints-editor.js';
import {findSavedRelation,findSavedLink,joinSides,resolveSavedField,validateSettings} from './select-settings.js';
import {knownType,typeSql} from './data-types.js';
import {mountTemplatePanel} from './template-panel.js';

const token=document.querySelector('meta[name="pg-debugger-token"]')?.content;
let debugActive=false;
function changed(){document.dispatchEvent(new CustomEvent('metadata-context-change'));}
document.addEventListener('debugstate',event=>{if(debugActive!==event.detail.active){debugActive=event.detail.active;changed();}});

export function mountSelectHelper(root,project,applyQuery,applyModel,selectionBlock,onSettings,onTemplate){
  const section=document.createElement('section');section.className='select-helper';
  root.append(section);
  const inputs=[],viewContext=context;
  let state=null,details=null,joined=null,extra=[],busy=false,restoring=false,restoreError=false;
  function field(caption,parent=section,wide=false){
    const label=document.createElement('label');label.textContent=caption;
    if(wide)label.className='wide';
    const select=document.createElement('select');label.append(select);parent.append(label);inputs.push(select);return select;
  }
  function accordion(caption,parent=section){
    const panel=document.createElement('details'),summary=document.createElement('summary'),body=document.createElement('div');
    panel.className='select-options';summary.textContent=caption;
    panel.append(summary,body);parent.append(panel);return {panel,body};
  }
  function options(select,rows,placeholder,previous){select.replaceChildren();if(placeholder)rows=[['',placeholder],...rows];for(const [value,text]of rows){const option=document.createElement('option');option.value=value;option.textContent=text;select.append(option);}if(previous!==undefined && rows.some(([value])=>value===previous))select.value=previous;}
  const schema=field('Схема таблицы'),relation=field('Таблица или представление');
  const resultFields=document.createElement('div');resultFields.className='select-result-fields';section.append(resultFields);
  const mode=field('Что получить',resultFields),column=field('Поле результата',resultFields);options(mode,selectModes);
  const filterGroup=accordion('Фильтр по параметру');
  const filter=field('Поле фильтра',filterGroup.body),parameter=field('Параметр для сравнения',filterGroup.body);
  const joins=accordion('Связанные таблицы (JOIN)');
  const link=field('Связь по внешнему ключу',joins.body,true),joinKind=field('Способ соединения',joins.body,true);
  options(joinKind,[['inner','INNER JOIN - совпавшие строки'],['left','LEFT JOIN - все строки основной таблицы']]);
  const moreJoins=accordion('Добавить ещё связь',joins.body);
  const extraFrom=field('Исходная таблица',moreJoins.body,true);
  const extraLink=field('Следующий внешний ключ',moreJoins.body,true);
  const extraKind=field('Тип следующего JOIN',moreJoins.body,true);
  options(extraKind,[['inner','INNER JOIN'],['left','LEFT JOIN']]);
  const extraAdd=document.createElement('button');extraAdd.type='button';extraAdd.textContent='Добавить JOIN';moreJoins.body.append(extraAdd);
  const extraRemove=document.createElement('button');extraRemove.type='button';extraRemove.textContent='Убрать последний JOIN';joins.body.append(extraRemove);
  const extraList=document.createElement('ol');extraList.className='select-join-list';joins.body.append(extraList);
  const add=document.createElement('button');add.type='button';add.textContent='Составить SELECT';section.append(add);
  const extras=accordion('Дополнительно');
  const info=document.createElement('details'),summary=document.createElement('summary');summary.textContent='Поля и связи';
  const structure=document.createElement('div');info.append(summary,structure);extras.body.append(info);
  const editor=document.createElement('div');editor.className='model-editor-slot';extras.body.append(editor);
  const refresh=document.createElement('button');refresh.type='button';refresh.textContent='Обновить структуру';extras.body.append(refresh);
  const message=document.createElement('p');message.className='hint';message.setAttribute('role','status');section.append(message);
  const templatePanel=mountTemplatePanel(extras.body,()=>({project,settings:capture(),context:{relation:selectedRelation(),details,joined,extra}}),next=>onTemplate?.(next));
  const alive=()=>section.isConnected && context===viewContext;
  const selectedRelation=()=>state?.relations.find(r=>String(r.oid)===relation.value);
  function controls(){
    const disabled=!context || !token || busy || restoring || debugActive;
    for(const input of [...inputs,refresh])input.disabled=disabled;
    link.disabled=disabled || !details?.foreign_keys.length;joinKind.disabled=disabled || !joined;
    const full=extra.length>=3;
    const ready=!!details,count=mode.value==='count';
    resultFields.hidden=!ready;
    column.parentElement.hidden=count;
    filterGroup.panel.hidden=!ready||(parameter.options.length<=1&&!filter.value);
    parameter.parentElement.hidden=!filter.value;
    joins.panel.hidden=!details?.foreign_keys.length;
    joinKind.parentElement.hidden=!link.value;
    moreJoins.panel.hidden=!joined||full;
    extraRemove.hidden=!extra.length;
    info.hidden=editor.hidden=!ready;
    add.hidden=!ready;
    extraFrom.disabled=disabled||!joined||full;
    extraLink.disabled=extraFrom.disabled||!tableAt(extraFrom.value)?.details.foreign_keys.length;
    extraKind.disabled=extraLink.disabled;
    extraAdd.disabled=extraLink.disabled||extraLink.value==='';
    extraRemove.disabled=disabled||!extra.length;
    column.disabled=disabled || count || !ready;
    parameter.disabled=disabled || filter.value==='' || !details;
    editor.querySelectorAll('input,select,textarea,button').forEach(input=>input.disabled=disabled);
    const countAllowed=selectedRelation()?.can_select_all && (!joined || joined.relation.can_select_all) && extra.every(item=>item.relation.can_select_all);
    add.disabled=disabled || restoreError || !ready || (link.value!=='' && !joined) ||
      (count?!countAllowed:!column.value) || (!!filter.value&&!parameter.value);
    templatePanel.setDisabled(disabled || restoreError || !details || (link.value!=='' && !joined));
  }
  function renderRelations(){const previous=relation.value;options(relation,(state?.relations||[]).map(r=>[String(r.oid),`${tableCaption(project.model,r)} (${({r:'таблица',p:'таблица с секциями',v:'представление',m:'материализованное представление',f:'внешняя таблица'})[r.kind]})`]),'Выберите таблицу',previous);}
  function tableAt(side){
    const tables=[{relation:selectedRelation(),details},joined,...extra];
    const table=tables[joinSides.indexOf(side)];
    return table?.relation&&table.details?table:null;
  }
  const sideCaption=side=>{
    const table=tableAt(side)?.relation,index=joinSides.indexOf(side);
    return table?`${index===0?'Основная':'JOIN '+index}: ${table.schema}.${table.name}`:side;
  };
  const keyOptions=keys=>keys.map((key,index)=>[String(index),`${key.name} → ${key.referenced_schema}.${key.referenced_table}`]);
  function renderExtraKeys(){
    options(extraLink,keyOptions(tableAt(extraFrom.value)?.details.foreign_keys||[]),'Выберите внешний ключ',extraLink.value);
  }
  function renderExtra(){
    const sides=joined?joinSides.slice(0,2+extra.length):[];
    options(extraFrom,sides.map(side=>[side,sideCaption(side)]),'Выберите исходную таблицу',extraFrom.value);
    renderExtraKeys();extraList.replaceChildren();
    for(const [i,item] of extra.entries()){
      const row=document.createElement('li');
      row.textContent=`${item.kind.toUpperCase()} JOIN · ${sideCaption(item.from)} → ${sideCaption(joinSides[i+2])}`;
      extraList.append(row);
    }
    extraList.hidden=!extra.length;
  }
  function availableColumns(){
    return joinSides.flatMap(side=>{
      const table=tableAt(side);
      return table?table.details.columns.map(column=>({side,relation:table.relation,column})):[];
    });
  }
  function columnOptions(rows){return rows.map(item=>[`${item.side}:${item.column.number}`,`${joined?sideCaption(item.side)+' · ':''}${columnCaption(project.model,item.relation,item.column)}`]);}
  function renderChoices(){
    templatePanel.invalidate();
    renderExtra();
    const previousColumn=column.value,previousFilter=filter.value,previousParameter=parameter.value;
    const rows=availableColumns();
    const resultColumns=mode.value==='count'?[]:columnOptions(rows.filter(item=>columnAllowed(mode.value,item.column)));
    options(column,resultColumns,resultColumns.length?'Выберите поле':'Подходящих полей нет',previousColumn);
    options(filter,columnOptions(rows),'Без фильтра',previousFilter);
    options(parameter,project.parameters.filter(p=>knownType(p.type)).map(p=>[p.name,`${p.name}: ${typeSql(p.type)}`]),'Выберите параметр',previousParameter);
    renderStructure();controls();
  }
  function renderStructure(){
    structure.replaceChildren();editor.replaceChildren();
    function appendTable(r,d){
      const heading=document.createElement('h4');heading.textContent=`${tableCaption(project.model,r)} (${r.schema}.${r.name})`;structure.append(heading);
      for(const [prefix,text]of [['Комментарий PostgreSQL',r.comment],['Описание проекта',tableHints(project.model,r)?.description]])if(text){const p=document.createElement('p');p.textContent=prefix+': '+text;structure.append(p);}
      const table=document.createElement('table');
      for(const c of d.columns){const hints=columnHints(project.model,r,c),row=document.createElement('tr');for(const text of [columnCaption(project.model,r,c),c.nullable?'NULL':'NOT NULL',[c.comment,hints?.description,(hints?.values||[]).map(v=>`${v.value} - ${v.label}`).join('; ')].filter(Boolean).join('\n')]){const cell=document.createElement('td');cell.textContent=text;row.append(cell);}table.append(row);}structure.append(table);
      for(const key of d.primary_keys){const p=document.createElement('p');p.textContent='Первичный ключ: '+key.columns.join(', ');structure.append(p);}
      for(const key of d.foreign_keys){const p=document.createElement('p');p.textContent=`${key.columns.join(', ')} → ${key.referenced_schema}.${key.referenced_table} (${key.referenced_columns.join(', ')})`;structure.append(p);}
    }
    if(details && selectedRelation()){
      appendTable(selectedRelation(),details);if(joined)appendTable(joined.relation,joined.details);
      for(const item of extra)appendTable(item.relation,item.details);
      if(!debugActive && applyModel)mountHintsEditor(editor,project,selectedRelation(),details,model=>{applyModel(model);renderRelations();renderChoices();message.textContent='Подсказки записаны в проект. Сохраните проект в браузере или скачайте файл.';});
    }
  }
  function saveLink(key,kind){
    return {constraint:key.name,kind,referencedSchema:key.referenced_schema,referencedTable:key.referenced_table,columns:[...key.columns],referencedColumns:[...key.referenced_columns]};
  }
  function capture(){
    const r=selectedRelation(),rows=availableColumns();
    function selectedField(value){const [side,number]=value.split(':');const item=rows.find(item=>item.side===side && item.column.number===Number(number));return item?{side,name:item.column.name}:null;}
    const key=link.value!==''?details?.foreign_keys[Number(link.value)]:null,selectedFilter=selectedField(filter.value);
    return validateSettings({schema:schema.value||project.schema,table:r?.name||null,mode:mode.value,column:selectedField(column.value),filter:selectedFilter?{...selectedFilter,parameter:parameter.value||null}:null,join:key?saveLink(key,joinKind.value):null,...(extra.length?{joins:extra.map(item=>({from:item.from,link:saveLink(item.foreignKey,item.kind)}))}:{})});
  }
  function remember(){templatePanel.invalidate();if(state && !restoring && !restoreError && alive() && onSettings)onSettings(capture());}
  function buildCurrent(){
    const [columnSide,columnNumber]=column.value.split(':'),[filterSide,filterNumber]=filter.value.split(':');
    return buildSelect({relation:selectedRelation(),columns:details.columns,mode:mode.value,columnNumber:Number(columnNumber),columnSide:columnSide||'source',filterNumber:filter.value?Number(filterNumber):null,filterSide:filterSide||'source',parameter:parameter.value,parameters:project.parameters,variables:project.variables,routineName:project.name,join:joined?{...joined,columns:joined.details.columns,kind:joinKind.value}:null,extra:extra.map(item=>({...item,columns:item.details.columns}))});
  }
  async function load(selectedSchema=schema.value || viewContext?.schema,oid=null,force=false){
    if(!alive() || !token || !context || busy || debugActive)return;
    busy=true;controls();message.textContent='Читаем структуру базы…';
    try{
      const result=await readMetadata({...viewContext,schema:selectedSchema||null,relation_oid:oid},alive,force);if(!alive())return;
      state=result;details=result.selected;joined=null;extra=[];
      options(schema,result.schemas.map(name=>[name,name]));schema.value=result.schema||'';
      renderRelations();relation.value=oid===null?'':String(oid);
      column.value=filter.value=parameter.value='';
      options(link,(details?.foreign_keys||[]).map((key,index)=>[String(index),`${key.name}: ${key.columns.join(', ')} → ${key.referenced_schema}.${key.referenced_table}`]),'Без JOIN');
      renderChoices();message.textContent=!oid&&!result.relations.length?'В схеме нет доступных таблиц и представлений.':'';
    }catch(error){if(alive()){state=null;details=null;joined=null;extra=[];structure.replaceChildren();editor.replaceChildren();options(relation,[],'Повторите обновление');options(column,[]);options(filter,[]);options(link,[],'Без JOIN');renderExtra();message.textContent=error.message;}if(restoring)throw error;}
    finally{busy=false;if(alive())controls();}
  }
  async function loadLink(){
    joined=null;extra=[];if(link.value===''){renderChoices();return;}
    const source=details,key=source.foreign_keys[Number(link.value)];if(!key)return;
    busy=true;controls();message.textContent='Читаем связанную таблицу…';
    try{
      const result=await readMetadata({...viewContext,schema:key.referenced_schema,relation_oid:key.referenced_oid},alive);
      if(!alive() || source!==details)return;
      const r=result.relations.find(r=>r.oid===key.referenced_oid);
      if(!r || !result.selected)throw Error('Связанная таблица недоступна. Обновите структуру.');
      joined={relation:r,details:result.selected,foreignKey:key};renderChoices();message.textContent='Связь загружена. ON использует все колонки внешнего ключа.';
    }catch(error){if(alive()){renderChoices();message.textContent=error.message;}if(restoring)throw error;}
    finally{busy=false;if(alive())controls();}
  };
  async function extend(from,key,kind){
    const result=await readMetadata({...viewContext,schema:key.referenced_schema,relation_oid:key.referenced_oid},alive);
    if(!alive())return;
    const relation=result.relations.find(item=>item.oid===key.referenced_oid);
    if(!relation||!result.selected)throw Error('Связанная таблица недоступна. SQL сохранён.');
    extra.push({from,kind,relation,details:result.selected,foreignKey:key});
  }
  extraFrom.onchange=()=>{extraLink.value='';renderExtraKeys();controls();};
  extraLink.onchange=controls;
  extraAdd.onclick=async()=>{
    const from=extraFrom.value,key=tableAt(from)?.details.foreign_keys[Number(extraLink.value)];
    if(extraAdd.disabled||!key)return;
    busy=true;controls();
    try{
      await extend(from,key,extraKind.value);if(!alive())return;
      renderChoices();remember();
      message.textContent='Связь добавлена. Нажмите «Составить SELECT», чтобы обновить запрос.';
    }catch(error){if(alive())message.textContent=error.message;}
    finally{busy=false;if(alive())controls();}
  };
  extraRemove.onclick=()=>{
    extra.pop();renderChoices();remember();
    message.textContent='Последняя связь убрана. Текст запроса сохранён.';
  };
  link.onchange=()=>{
    restoreError=false;
    // Номер поля другой таблицы может совпасть с прежним.
    if(column.value&&!column.value.startsWith('source:'))column.value='';
    if(filter.value&&!filter.value.startsWith('source:'))filter.value=parameter.value='';
    moreJoins.panel.open=false;
    const pending=loadLink();remember();pending.finally(remember);
  };
  joinKind.onchange=remember;
  column.onchange=()=>{controls();remember();};
  parameter.onchange=()=>{controls();remember();};
  schema.onchange=()=>{restoreError=false;onSettings?.({schema:schema.value,table:null,mode:mode.value,column:null,filter:null,join:null});load(schema.value).finally(remember);};
  relation.onchange=()=>{restoreError=false;const r=selectedRelation();onSettings?.({schema:schema.value,table:r?.name||null,mode:mode.value,column:null,filter:null,join:null});load(schema.value,r?.oid||null).finally(remember);};
  mode.onchange=()=>{renderChoices();remember();if(mode.value==='count' && add.disabled)message.textContent='COUNT(*) требует SELECT на все таблицы запроса.';};
  filter.onchange=()=>{
    if(!filter.value)parameter.value='';
    controls();remember();const [side,number]=filter.value.split(':');
    const item=availableColumns().find(item=>item.side===side && item.column.number===Number(number));
    const suggestions=item?columnHints(project.model,item.relation,item.column)?.values:[];
    if(suggestions?.length)message.textContent='Подсказки проекта: '+suggestions.map(v=>`${v.value} - ${v.label}`).join('; ');
  };
  refresh.onclick=()=>{clearMetadataCache();restore();};
  const reset=document.createElement('button');reset.type='button';reset.textContent='Сбросить выбор';extras.body.append(reset);
  inputs.push(reset);
  reset.onclick=()=>{restoreError=false;onSettings?.(null);load(viewContext?.schema||project.schema).then(()=>{message.textContent='Выбор сброшен. Текст SQL сохранён.';});};
  async function restore(){
    const saved=selectionBlock?.assistant;
    if(!saved){await load();return;}
    restoring=true;restoreError=false;controls();
    try{
      await load(saved.schema);if(!alive())return;
      if(saved.table===null){mode.value=saved.mode;message.textContent='Сохранённый выбор схемы восстановлен.';return;}
      const source=findSavedRelation(saved,state);await load(saved.schema,source.oid);if(!alive())return;
      mode.value=saved.mode;
      if(saved.join){const key=findSavedLink(saved.join,details);link.value=String(details.foreign_keys.indexOf(key));joinKind.value=saved.join.kind;await loadLink();if(!alive())return;}
      for(const step of saved.joins||[]){
        const origin=tableAt(step.from);
        if(!origin)throw Error('Исходная таблица сохранённой связи недоступна. SQL сохранён.');
        const key=findSavedLink(step.link,origin.details);
        await extend(step.from,key,step.link.kind);if(!alive())return;
      }
      renderChoices();column.value=resolveSavedField(saved.column,details,joined,extra);filter.value=resolveSavedField(saved.filter,details,joined,extra);
      if(saved.column && saved.mode!=='count' && !column.value)throw Error('Тип сохранённого поля больше не подходит для операции. SQL сохранён.');
      if(saved.filter?.parameter && !project.parameters.some(p=>p.name===saved.filter.parameter))throw Error('Сохранённый входной параметр отсутствует. SQL сохранён.');
      parameter.value=saved.filter?.parameter||'';
      filterGroup.panel.open=!!saved.filter;
      joins.panel.open=!!saved.join;
      let equal=false;try{equal=buildCurrent()===selectionBlock.query;}catch{}
      message.textContent=equal?'Настройки SELECT восстановлены по структуре базы.':'Настройки SELECT восстановлены. Текст SQL отличается от выбора помощника и сохранён без изменений. Составление SELECT заменит этот текст.';
    }catch(error){if(alive()){restoreError=true;message.textContent=error.message;}}
    finally{restoring=false;if(alive())controls();}
  }
  add.onclick=()=>{
    try{
      const query=buildCurrent();remember();
      applyQuery(query);message.textContent='SELECT составлен.';
    }catch(error){message.textContent=error.message;}
  };
  message.textContent=!token || !context?'Подключитесь к PostgreSQL для выбора данных. Чтобы написать запрос вручную, выберите «Ввести SQL».':debugActive?'Завершите отладку для обновления структуры.':'';
  controls();
  let activated=false;
  return ()=>{
    if(activated||!context||!token||debugActive)return;
    activated=true;
    restore();
  };
}
