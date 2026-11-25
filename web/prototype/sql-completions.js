import {tableHints,columnHints} from './model-hints.js';
const part=/[\p{L}\p{N}_$]/u,first=/[\p{L}_]/u;
const keywords=new Set('WHERE JOIN LEFT RIGHT INNER OUTER FULL CROSS ON USING SET FROM GROUP ORDER LIMIT OFFSET RETURNING INTO VALUES UNION AND OR THEN LOOP END AS SELECT UPDATE DELETE INSERT'.split(' '));
export const quoteIdentifier=name=>'"'+name.replace(/"/g,'""')+'"';

// Нужен только контекст подсказок, не проверка SQL и не восстановление визуального графа.
function* tokens(source,end=source.length){
  let i=0;
  while(i<end){
    const start=i,c=source[i];
    if(/\s/.test(c)){i++;continue;}
    if(source.startsWith('--',i)){const next=source.indexOf('\n',i+2);if(next<0||next>=end){yield {kind:'blocked'};return;}i=next+1;continue;}
    if(source.startsWith('/*',i)){
      i+=2;let depth=1;
      while(i<end&&depth){if(source.startsWith('/*',i)){depth++;i+=2;}else if(source.startsWith('*/',i)){depth--;i+=2;}else i++;}
      if(depth){yield {kind:'blocked'};return;}continue;
    }
    if(c==="'"){
      i++;let closed=false;
      while(i<end){if(source[i]==="'"){if(source[i+1]==="'"&&i+1<end){i+=2;continue;}i++;closed=true;break;}if(source[i]==='\\'){i+=2;}else i++;}
      if(!closed){yield {kind:'blocked'};return;}continue;
    }
    if(c==='$'){
      const tag=source.slice(i,end).match(/^\$(?:[\p{L}_][\p{L}\p{N}_]*)?\$/u)?.[0];
      if(tag){const next=source.indexOf(tag,i+tag.length);if(next<0||next+tag.length>end){yield {kind:'blocked'};return;}i=next+tag.length;continue;}
    }
    if(c==='"'){
      i++;let name='',closed=false;
      while(i<end){if(source[i]==='"'){if(source[i+1]==='"'&&i+1<end){name+='"';i+=2;continue;}i++;closed=true;break;}name+=source[i++];}
      yield {kind:'id',name,start,end:i,quoted:true,closed};continue;
    }
    if(first.test(c)){
      i++;while(i<end&&part.test(source[i]))i++;
      yield {kind:'id',name:source.slice(start,i).toLowerCase(),start,end:i,quoted:false};continue;
    }
    i++;yield {kind:c,start,end:i};
  }
}

export function completionContext(source,pos=source.length){
  const current=[];let blocked=false;
  for(const token of tokens(source,pos)){
    if(token.kind==='blocked'){blocked=true;break;}
    if(token.kind===';')current.length=0;
    else{current.push(token);if(current.length>4096)current.shift();}
  }
  if(blocked)return null;
  let from=pos,word='',end=current.length;
  const last=current[end-1];
  if(last?.kind==='id'&&last.end===pos){from=last.start;word=last.name;end--;}
  const qualifier=[];
  while(end>=2&&current[end-1].kind==='.'&&current[end-2].kind==='id'){
    qualifier.unshift(current[end-2].name);end-=2;
  }
  return {from,word,qualifier,tokens:current};
}

export function declaredVariables(source){
  const result=[];let declaring=false,expectName=false,name=null;
  for(const token of tokens(source)){
    if(token.kind==='blocked')break;
    if(token.kind==='id'&&!token.quoted&&token.name==='declare'){declaring=true;expectName=true;name=null;continue;}
    if(!declaring)continue;
    if(token.kind==='id'&&!token.quoted&&token.name==='begin'){declaring=false;name=null;continue;}
    if(token.kind===';'){expectName=true;name=null;continue;}
    if(expectName&&token.kind==='id'){name=token.name;expectName=false;continue;}
    if(name&&token.kind==='id'){
      if(!token.quoted&&token.name==='constant')continue;
      result.push({name,type:token.name});name=null;if(result.length>=1000)break;
    }
  }
  return result;
}

function aliases(list){
  const result=new Map();
  for(let i=0;i<list.length;i++){
    if(list[i].kind!=='id'||list[i].quoted||!['from','join','update'].includes(list[i].name))continue;
    let j=i+1;const path=[];
    if(list[j]?.kind!=='id')continue;
    path.push(list[j++].name);
    while(list[j]?.kind==='.'&&list[j+1]?.kind==='id'){path.push(list[j+1].name);j+=2;}
    let alias=path[path.length-1];
    if(list[j]?.kind==='id'&&!list[j].quoted&&list[j].name==='as')j++;
    if(list[j]?.kind==='id'&&(list[j].quoted||!keywords.has(list[j].name.toUpperCase())))alias=list[j].name;
    if(path.length<=2)result.set(alias,path);
  }
  return result;
}

export function completionOptions({source,pos=source.length,schemas=[],relations=[],tables=[],parameters=[],variables,model}){
  const context=completionContext(source,pos);if(!context)return null;
  const options=[],seen=new Set();
  function add(name,type,detail,info,apply=quoteIdentifier(name)){
    const key=type+'\0'+apply;if(seen.has(key))return;seen.add(key);
    options.push({label:name,type,detail,info,apply});
  }
  const qualified=context.qualifier;
  if(!qualified.length){
    for(const parameter of parameters)if(parameter.name)add(parameter.name,'variable',`${parameter.mode||'IN'} ${parameter.type||''}`,'Параметр выбранной функции или процедуры.');
    for(const variable of variables||declaredVariables(source))if(variable.name)add(variable.name,'variable',variable.type||'Переменная','Локальная переменная. Распознавание DECLARE ограничено; синтаксис проверяет PostgreSQL.');
    for(const schema of schemas)add(schema,'namespace','Схема PostgreSQL','Доступна текущему подключению.');
    for(const relation of relations){const hint=tableHints(model,relation);add(relation.name,'class',`${relation.schema}${hint?.label?' · '+hint.label:''}`,[relation.comment,hint?.description].filter(Boolean).join('\n'),`${quoteIdentifier(relation.schema)}.${quoteIdentifier(relation.name)}`);}
  }
  let candidates=[];
  if(qualified.length===1){
    const path=aliases(context.tokens).get(qualified[0]);
    if(path)candidates=tables.filter(t=>t.relation.name===path[path.length-1]&&(path.length===1||t.relation.schema===path[0]));
    else if(schemas.includes(qualified[0])){
      for(const relation of relations.filter(r=>r.schema===qualified[0]))add(relation.name,'class',relation.schema,tableHints(model,relation)?.description||relation.comment||'');
    }else candidates=tables.filter(t=>t.relation.name===qualified[0]);
  }else if(qualified.length===2)candidates=tables.filter(t=>t.relation.schema===qualified[0]&&t.relation.name===qualified[1]);
  // При неоднозначном имени не угадываем таблицу. Подсказки не доказывают search_path.
  if(candidates.length===1){
    const {relation,details}=candidates[0];
    for(const column of details.columns){const hint=columnHints(model,relation,column);add(column.name,'property',`${column.data_type}${hint?.label?' · '+hint.label:''}`,[`${relation.schema}.${relation.name}.${column.name}`,column.comment,hint?.description,(hint?.values||[]).map(v=>`${v.value} - ${v.label}`).join('; ')].filter(Boolean).join('\n'));}
  }
  return {...context,options};
}
