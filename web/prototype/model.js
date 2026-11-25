import {constructorTypes,declarationIssues,knownType,typeSql} from './data-types.js';
// Структура блоков хранится отдельно от интерфейса.
import {emptyModel,validateModel,normalizeModel} from './model-hints.js';
import {validateSettings} from './select-settings.js';
import {MAX_BLOCK_DEPTH,validateWorkingProject,normalizeProjectTypes} from './working-draft.js';
export const types = constructorTypes;
export const labels = {assign:'Присваивание',select:'Запрос SELECT',if:'Условие IF',while:'Цикл WHILE',for:'Цикл FOR',return:'Возврат RETURN',sql:'Ручной SQL'};
export const isLoop = item => item?.type === 'while' || item?.type === 'for';
export function childBlocks(item){
  if(item.type==='if')return [['then','Да'],['else','Нет']];
  return isLoop(item)?[['body','Тело цикла']]:[];
}

// Учитываем и пустые ветки: после вставки их тоже нужно сохранить.
export function fitsAtDepth(item, depth) {
  const branches = childBlocks(item);
  return (
    depth <= MAX_BLOCK_DEPTH &&
    branches.every(
      ([key]) => depth < MAX_BLOCK_DEPTH && item[key].every((child) => fitsAtDepth(child, depth + 1)),
    )
  );
}

const ident = value => typeof value === 'string' && /^[a-z_][a-z_0-9]{0,62}$/i.test(value);
const quote = value => `"${value}"`;
const defaults = {
  assign:()=>({target:'running_total',expression:'running_total + 1'}),
  select:()=>({query:'SELECT 1',target:'running_total'}),
  if:()=>({condition:'input_value > 0',then:[],else:[]}),
  while:()=>({condition:'',body:[]}),
  for:()=>({iterator:'i',lower:'1',upper:'input_value',body:[]}),
  return:()=>({expression:'running_total'}),
  sql:()=>({sql:"RAISE NOTICE 'result=%', running_total;"}),
};
export const block = type => ({id:globalThis.crypto.randomUUID(), type, ...defaults[type]()});
export function emptyProject(schema='public'){
  return {schemaVersion:3,model:emptyModel(),kind:'function',schema,name:'',returnType:'integer',parameters:[],variables:[],blocks:[]};
}
export function example(kind='function') {
  return {schemaVersion:3,model:emptyModel(),kind,schema:'public',name:kind==='function'?'visual_smoke':'visual_smoke_proc',returnType:'integer',parameters:[{name:'input_value',type:'integer'}],variables:[{name:'running_total',type:'integer',initial:'input_value'}],blocks:[block('assign'),{...block('assign'),expression:'running_total * 2'},block(kind==='function'?'return':'sql')]};
}
export function validate(project) {
  const errors=[];
  if (!project || ![1,2,3].includes(project.schemaVersion)) return ['Неподдерживаемый формат проекта. Ожидается версия 1, 2 или 3.'];
  if(project.schemaVersion>=2){try{validateModel(project.model);}catch(error){errors.push(error.message);}}
  else if(project.model!==undefined)errors.push('Настройки модели требуют версию проекта 2.');
  if (!['function','procedure'].includes(project.kind)) errors.push('Выберите функцию или процедуру.');
  if (!ident(project.schema)||!ident(project.name)) errors.push('Схема и имя: латинские буквы, цифры и знак подчёркивания; первый символ - буква или подчёркивание.');
  if (!knownType(project.returnType)) errors.push('Неизвестный тип результата.');
  for (const key of ['parameters','variables']) {
    if (!Array.isArray(project[key]) || project[key].length>100) errors.push('Некорректный список параметров или переменных.');
  }
  for(const issue of declarationIssues(project))errors.push(`${issue.key==='parameters'?'Параметр':'Переменная'} ${issue.index+1}: ${issue.message}`);
  const ids=new Set(); let count=0;
  function walk(list,depth=0) {
    if (!Array.isArray(list)||depth>MAX_BLOCK_DEPTH) {errors.push('Неверная структура или слишком глубокие ветви.');return;}
    for (const b of list) {
      if (++count>500) {errors.push('Допускается до 500 блоков.');return;}
      if (!b || typeof b.id!=='string'||ids.has(b.id)||!labels[b.type]) {errors.push('Неизвестный блок или повторяющийся идентификатор.');continue;}
      ids.add(b.id);
      if(b.type==='select' && b.assistant!==undefined && b.assistant!==null){try{if(project.schemaVersion<3)throw Error('Настройки SELECT требуют версию проекта 3.');validateSettings(b.assistant);}catch(error){errors.push(error.message);}}
      if (['assign','select'].includes(b.type)&&(!ident(b.target)||!project.variables?.some(v=>v.name===b.target))) errors.push('Результат блока должен записываться в объявленную переменную.');
      const field={assign:'expression',select:'query',if:'condition',while:'condition',sql:'sql'}[b.type];
      if (field&&(typeof b[field]!=='string'||!b[field].trim())) errors.push(`Заполните блок «${labels[b.type]}».`);
      if (b.type==='select' && typeof b.query==='string' && !/^select\b/i.test(b.query.trim())) errors.push('В блоке SELECT запрос должен начинаться с SELECT.');
      if (b.type==='return' && project.kind==='function' && (typeof b.expression!=='string'||!b.expression.trim())) errors.push('Укажите возвращаемое выражение.');
      if(b.type==='for'&&(!ident(b.iterator)||[b.lower,b.upper].some(value=>typeof value!=='string'||!value.trim())))errors.push('Заполните имя счётчика и границы цикла FOR.');
      for(const [key] of childBlocks(b))walk(b[key],depth+1);
    }
  }
  walk(project.blocks);
  function returns(list) {return Array.isArray(list)&&list.some(b=>b?.type==='return'||(b?.type==='if'&&returns(b.then)&&returns(b.else)));}
  if (!errors.length && project.kind==='function'&&!returns(project.blocks)) errors.push('Добавьте RETURN для всех путей функции.');
  return [...new Set(errors)];
}
export function generate(project) {
  const errors=validate(project); if(errors.length) return {errors,sql:'',sourceMap:{}};
  const lines=[],sourceMap={}; const emit=s=>lines.push(s);
  emit(`CREATE OR REPLACE ${project.kind==='function'?'FUNCTION':'PROCEDURE'} ${quote(project.schema)}.${quote(project.name)}(`);
  project.parameters.forEach((p,i)=>emit(`    ${quote(p.name)} ${typeSql(p.type)}${i<project.parameters.length-1?',':''}`));
  emit(')'); if(project.kind==='function') emit(`RETURNS ${typeSql(project.returnType)}`); emit('LANGUAGE plpgsql');
  const body=[];
  const add=s=>body.push(...s.split('\n'));
  if(project.variables.length) {add('DECLARE');project.variables.forEach(v=>`    ${quote(v.name)} ${typeSql(v.type)}${v.initial?.trim()?` := ${v.initial}`:''};`.split('\n').forEach(add));}
  add('BEGIN');
  // Переводим строки тела в строки полного SQL.
  function walk(list,depth=1) {
    if (!list.length) add('    '.repeat(depth)+'NULL;');
    for(const b of list) {
      const start=body.length+1,pad='    '.repeat(depth);
      const expr=s=>s.trim().replace(/;\s*$/,'');
      if(b.type==='assign') add(`${pad}${quote(b.target)} := ${expr(b.expression)};`);
      if(b.type==='select') {
        // Подзапрос даёт одно значение для переменной.
        add(`${pad}SELECT * INTO ${quote(b.target)}`);
        add(`${pad}FROM (`);expr(b.query).split('\n').forEach(s=>add(pad+'    '+s));add(`${pad}) AS selected_value;`);
      }
      if(b.type==='sql') b.sql.trim().replace(/;\s*$/,'').split('\n').forEach((s,i,a)=>add(pad+s+(i===a.length-1?';':'')));
      if(b.type==='return') add(`${pad}RETURN${project.kind==='function'?' '+expr(b.expression):''};`);
      if(b.type==='if') {add(`${pad}IF ${expr(b.condition)} THEN`);walk(b.then,depth+1);add(`${pad}ELSE`);walk(b.else,depth+1);add(`${pad}END IF;`);}
      if(isLoop(b)){
        const head=b.type==='while'?`WHILE ${expr(b.condition)}`:`FOR ${quote(b.iterator)} IN (${expr(b.lower)})..(${expr(b.upper)})`;
        add(`${pad}${head} LOOP`);
        walk(b.body,depth+1);
        add(`${pad}END LOOP;`);
      }
      sourceMap[b.id]={bodyStart:start+1,bodyEnd:body.length+1};
    }
  }
  walk(project.blocks);add('END;');
  let delimiter='$body$';while(body.join('\n').includes(delimiter)) delimiter=delimiter.slice(0,-1)+'_$';
  emit(`AS ${delimiter}`);const offset=lines.length;
  body.forEach(emit);emit(`${delimiter};`);
  for(const range of Object.values(sourceMap)){range.start=offset+range.bodyStart-1;range.end=offset+range.bodyEnd-1;}
  return {errors:[],sql:lines.join('\n'),sourceMap,body:'\n'+body.join('\n')+'\n'};
}
export function parseDraft(text) {
  if(text.length>2_000_000) throw Error('Файл проекта слишком большой.');
  const project=validateWorkingProject(JSON.parse(text)), errors=validate(project);
  if(errors.length) throw Error(errors.join('\n'));
  normalizeProjectTypes(project);
  if(project.schemaVersion===1)project.model=emptyModel();
  project.schemaVersion=3;
  project.model=normalizeModel(project.model);
  return project;
}
