// Только переносимый выбор пользователя. SQL остаётся отдельным полем блока.
const text=v=>typeof v==='string' && v.length>0 && new TextEncoder().encode(v).length<=63 && !v.includes('\0');
const shape=(v,keys)=>v && typeof v==='object' && !Array.isArray(v) && Object.keys(v).every(k=>keys.includes(k));
export const joinSides=['source','joined','join_2','join_3','join_4'];
const field=v=>shape(v,['side','name']) && joinSides.includes(v.side) && text(v.name);
export function validateSettings(v){
  const fail=()=>{throw Error('Неверные настройки SELECT-помощника.');};
  if(!shape(v,['schema','table','mode','column','filter','join','joins']) || !text(v.schema) || !(v.table===null || text(v.table)) || !['value','count','sum','min','max'].includes(v.mode))fail();
  if(v.column!==null && !field(v.column))fail();
  if(v.filter!==null && (!shape(v.filter,['side','name','parameter']) || !field({side:v.filter.side,name:v.filter.name}) || !(v.filter.parameter===null || text(v.filter.parameter))))fail();
  const validLink=j=>shape(j,['constraint','kind','referencedSchema','referencedTable','columns','referencedColumns']) && text(j.constraint) && ['inner','left'].includes(j.kind) && text(j.referencedSchema) && text(j.referencedTable) && Array.isArray(j.columns) && Array.isArray(j.referencedColumns) && j.columns.length>0 && j.columns.length<=1600 && j.columns.length===j.referencedColumns.length && j.columns.every(text) && j.referencedColumns.every(text);
  if(v.join!==null && !validLink(v.join))fail();
  if(Object.prototype.hasOwnProperty.call(v,'joins') && (!Array.isArray(v.joins) || v.joins.length>3 || (v.joins.length&&!v.join)))fail();
  for(const [index,step] of (v.joins||[]).entries()){
    if(!shape(step,['from','link']) || !joinSides.slice(0,index+2).includes(step.from) || !validLink(step.link))fail();
  }
  const sides=joinSides.slice(0,1+Number(!!v.join)+(v.joins?.length||0));
  if([v.column,v.filter].some(f=>f&&!sides.includes(f.side)))fail();
  return v;
}
export function findSavedRelation(settings,metadata){
  const relation=metadata.relations.find(r=>r.schema===settings.schema && r.name===settings.table);
  if(!relation)throw Error(`Сохранённая таблица ${settings.schema}.${settings.table} не найдена или недоступна. SQL сохранён.`);
  return relation;
}
export function findSavedLink(saved,details){
  if(!saved)return null;
  const key=details.foreign_keys.find(k=>k.name===saved.constraint);
  if(!key || key.referenced_schema!==saved.referencedSchema || key.referenced_table!==saved.referencedTable || JSON.stringify(key.columns)!==JSON.stringify(saved.columns) || JSON.stringify(key.referenced_columns)!==JSON.stringify(saved.referencedColumns))throw Error('Сохранённый внешний ключ удалён или изменён. SQL сохранён; выберите связь заново.');
  return key;
}
export function resolveSavedField(saved,details,joined,extra=[]){
  if(!saved)return '';
  const tables=[details,joined?.details,...extra.map(item=>item.details)];
  const column=tables[joinSides.indexOf(saved.side)]?.columns.find(c=>c.name===saved.name);
  if(!column)throw Error(`Сохранённое поле ${saved.name} не найдено или недоступно. SQL сохранён.`);
  return `${saved.side}:${column.number}`;
}
