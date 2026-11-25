// Настройки связываются с настоящими именами. Подписи не участвуют в SQL.
const bytes = value => new TextEncoder().encode(value).length;
const text = (value, max) => typeof value==='string' && !value.includes('\0') && bytes(value)<=max;
const name = value => text(value,63) && value.length>0;
const object = (value, allowed) => value && typeof value==='object' && !Array.isArray(value) && Object.keys(value).every(key=>allowed.includes(key));
const optionalText = (value,max) => value===undefined || text(value,max);
export const emptyModel = () => ({name:'Модель проекта',tables:[]});
export function formatHintLines(values){return values.map(v=>v.value.startsWith('[') || /[|\r\n]/.test(v.value) || /[\r\n]/.test(v.label) || v.value.trim()!==v.value || v.label.trim()!==v.label?JSON.stringify([v.value,v.label]):`${v.value} | ${v.label}`).join('\n');}
export function parseHintLines(input){
  return input.split('\n').filter(line=>line.trim()).map((line,index)=>{
    if(line.trim().startsWith('[')){
      let pair;try{pair=JSON.parse(line);}catch{throw Error(`Строка ${index+1}: неверная JSON-пара значений.`);}
      if(!Array.isArray(pair) || pair.length!==2 || pair.some(value=>typeof value!=='string'))throw Error(`Строка ${index+1}: нужны два текста - значение и подпись.`);
      return {value:pair[0],label:pair[1]};
    }
    const separator=line.indexOf('|');if(separator<0)throw Error(`Строка ${index+1}: используйте значение | подпись.`);
    return {value:line.slice(0,separator).trim(),label:line.slice(separator+1).trim()};
  });
}
export function normalizeModel(model){return {name:model.name,tables:model.tables.map(t=>({...t,label:t.label||'',description:t.description||'',columns:t.columns.map(c=>({...c,label:c.label||'',description:c.description||'',values:c.values||[]}))}))};}
export function validateModel(model) {
  const fail=()=>{throw Error('Неверные настройки модели: проверьте имена, размеры и повторы.');};
  if(!object(model,['name','tables']) || !text(model.name,128) || !Array.isArray(model.tables) || model.tables.length>200)fail();
  let total=0;const tables=new Set();
  for(const table of model.tables){
    if(!object(table,['schema','name','label','description','columns']) || !name(table.schema) || !name(table.name) || !optionalText(table.label,256) || !optionalText(table.description,4096) || !Array.isArray(table.columns))fail();
    const key=JSON.stringify([table.schema,table.name]);if(tables.has(key) || (total+=table.columns.length)>2000)fail();tables.add(key);
    const columns=new Set();
    for(const column of table.columns){
      if(!object(column,['name','label','description','values']) || !name(column.name) || !optionalText(column.label,256) || !optionalText(column.description,4096) || columns.has(column.name))fail();columns.add(column.name);
      const values=column.values===undefined?[]:column.values;
      if(!Array.isArray(values) || values.length>100)fail();const seen=new Set();
      for(const value of values){if(!object(value,['value','label']) || !text(value.value,512) || !text(value.label,256) || seen.has(value.value))fail();seen.add(value.value);}
    }
  }
  return model;
}
export const tableHints=(model,relation)=>model?.tables.find(t=>t.schema===relation.schema && t.name===relation.name);
export const columnHints=(model,relation,column)=>tableHints(model,relation)?.columns.find(c=>c.name===column.name);
export function tableCaption(model,relation){const label=tableHints(model,relation)?.label;return label?`${label} - ${relation.name}`:relation.name;}
export function columnCaption(model,relation,column){const label=columnHints(model,relation,column)?.label;return `${label?label+' - ':''}${column.name}: ${column.data_type}`;}
export function updateHints(model,relation,column,tablePatch,columnPatch){
  const next=JSON.parse(JSON.stringify(model||emptyModel()));
  let table=tableHints(next,relation);
  if(!table){table={schema:relation.schema,name:relation.name,label:'',description:'',columns:[]};next.tables.push(table);}
  Object.assign(table,tablePatch);
  if(column){let item=table.columns.find(c=>c.name===column.name);if(!item){item={name:column.name};table.columns.push(item);}Object.assign(item,columnPatch);}
  return validateModel(next);
}
