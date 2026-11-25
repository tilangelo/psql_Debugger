import {validateModel,normalizeModel,emptyModel} from './model-hints.js';
import {validateSettings} from './select-settings.js';
import {namedType} from './data-types.js';
export const MAX_BLOCK_DEPTH=12;

const bytes=text=>new TextEncoder().encode(text).length;
const text=(value,max=1_000_000)=>typeof value==='string'&&!value.includes('\0')&&bytes(value)<=max;
function object(value,allowed){return value&&typeof value==='object'&&!Array.isArray(value)&&Object.keys(value).every(k=>allowed.includes(k));}
function requireShape(ok){if(!ok)throw Error('Повреждённый рабочий черновик: проверьте формат, поля и размеры.');}

export function normalizeProjectTypes(project){
  for(const type of [project.returnType,...project.parameters.map(p=>p.type),...project.variables.map(v=>v.type)]){
    if(namedType(type)&&type.array===false)delete type.array;
  }
}

// Проверяет переносимый формат, но разрешает незаконченные выражения и временные ошибки имён.
export function validateWorkingProject(project){
  requireShape(object(project,['schemaVersion','model','kind','schema','name','returnType','parameters','variables','blocks']));
  requireShape([1,2,3].includes(project.schemaVersion)&&['function','procedure'].includes(project.kind));
  for(const field of ['schema','name'])requireShape(text(project[field]));
  requireShape(text(project.returnType)||namedType(project.returnType));
  if(project.schemaVersion>=2)validateModel(project.model);else requireShape(project.model===undefined);
  for(const key of ['parameters','variables']){
    requireShape(Array.isArray(project[key])&&project[key].length<=100);
    for(const item of project[key]){requireShape(object(item,key==='parameters'?['name','type']:['name','type','initial']));requireShape(text(item.name)&&(text(item.type)||namedType(item.type)));if(item.initial!==undefined)requireShape(text(item.initial));}
  }
  let count=0;const ids=new Set();
  const fields={assign:['target','expression'],select:['target','query','assistant'],if:['condition','then','else'],while:['condition','body'],for:['iterator','lower','upper','body'],return:['expression'],sql:['sql']};
  function walk(list,depth=0){
    requireShape(Array.isArray(list)&&depth<=MAX_BLOCK_DEPTH);
    for(const item of list){
      requireShape(item&&Object.hasOwnProperty.call(fields,item.type)&&++count<=500);
      requireShape(object(item,['id','type',...fields[item.type]])&&text(item.id,256)&&item.id.length>0&&!ids.has(item.id));ids.add(item.id);
      for(const key of fields[item.type]){
        if(key==='then'||key==='else'||key==='body')walk(item[key],depth+1);
        else if(key==='assistant'){if(item.assistant!=null){requireShape(project.schemaVersion>=3);validateSettings(item.assistant);}}
        else if(item.type==='return'&&key==='expression'&&item[key]===undefined)continue;
        else requireShape(text(item[key]));
      }
    }
  }
  walk(project.blocks);requireShape(bytes(JSON.stringify(project))<=2_000_000);
  return project;
}
export function parseWorkingSnapshot(value){
  requireShape(object(value,['project','selected','parametersText','variablesText','branch','declarationsVersion']));
  requireShape(value.declarationsVersion===undefined||value.declarationsVersion===2);
  const project=validateWorkingProject(value.project);
  requireShape(value.selected===null||text(value.selected,256));
  requireShape(text(value.parametersText)&&text(value.variablesText)&&['after','before','then','else','body'].includes(value.branch));
  const next=JSON.parse(JSON.stringify(value));
  normalizeProjectTypes(next.project);
  if(project.schemaVersion===1)next.project.model=emptyModel();
  next.project.schemaVersion=3;next.project.model=normalizeModel(next.project.model);
  return next;
}
export function validateBodyRecovery(value){
  requireShape(object(value,['target','definition','body']));
  const t=value.target;requireShape(object(t,['host','port','database','user','schema','name','signature','kind']));
  requireShape(text(t.host,128)&&Number.isInteger(t.port)&&t.port>0&&t.port<=65535);
  for(const key of ['database','user','schema','name'])requireShape(text(t[key],8192)&&t[key].length>0);
  requireShape(text(t.signature,8192));
  requireShape(['Функция','Процедура'].includes(t.kind)&&text(value.definition)&&text(value.body));
  return JSON.parse(JSON.stringify(value));
}
export function encodeWorkingFile(value){const text=JSON.stringify({format:'pgdebug-working-project',version:1,snapshot:parseWorkingSnapshot(value)},null,2);requireShape(bytes(text)<=2_200_000);return text;}
export function parseWorkingFile(text){
  requireShape(typeof text==='string'&&bytes(text)<=2_200_000);
  const file=JSON.parse(text);requireShape(object(file,['format','version','snapshot'])&&file.format==='pgdebug-working-project'&&file.version===1);
  return parseWorkingSnapshot(file.snapshot);
}
