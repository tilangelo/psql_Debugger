import {block,generate} from './model.js';
import {emptyModel} from './model-hints.js';
import {builtinArrayType,typeSql} from './data-types.js';

const sqlTypes=new Map([
  ['integer',['integer','pg_catalog.int4']],['text',['text','pg_catalog.text']],
  ['boolean',['boolean','pg_catalog.bool']],['bigint',['bigint','pg_catalog.int8']],
  ['numeric',['numeric','pg_catalog.numeric']],['date',['date','pg_catalog.date']],
  ['timestamp',['timestamp','timestamp without time zone','pg_catalog.timestamp']],
  ['uuid',['uuid','pg_catalog.uuid']],
  ['smallint',['smallint','pg_catalog.int2']],['real',['real','pg_catalog.float4']],
  ['double precision',['double precision','pg_catalog.float8']],['varchar',['varchar','character varying','pg_catalog.varchar']],
  ['json',['json','pg_catalog.json']],['jsonb',['jsonb','pg_catalog.jsonb']],
  ['timestamptz',['timestamptz','timestamp with time zone','pg_catalog.timestamptz']],
  ['time',['time','time without time zone','pg_catalog.time']],
  ['interval',['interval','pg_catalog.interval']],['bytea',['bytea','pg_catalog.bytea']],
]);
const identifier='[a-zA-Z_][a-zA-Z_0-9]{0,62}';
const pgIdentifier=new RegExp(`^(?:"(${identifier})"|(${identifier}))$`);
const readName=value=>{const match=pgIdentifier.exec(value);return match?.[1]||match?.[2];};
function readType(value){
  for(const [type,spellings] of sqlTypes){
    if(spellings.includes(value))return type;
    const array=builtinArrayType(type);
    if(value===typeSql(array)||spellings.some(spelling=>value===spelling+'[]'))return array;
  }
}
const unsupported=()=>Error('Поддерживаются стандартные определения PL/pgSQL с блоками конструктора. Для этого объекта используйте редактор тела или откройте сохранённый .pgflow.json.');

export function restoreGeneratedProject(catalog){
  const {selected,schema,routines}=catalog||{};
  const routine=routines?.find(item=>item.oid===selected?.oid);
  if(!routine || typeof selected?.source!=='string' || selected.source.length>500_000)throw unsupported();
  const source=selected.source.replace(/\r\n/g,'\n');
  const signature=selected.definition.replace(/\r\n/g,'\n').split('\n');
  // Берём свойства, которые конструктор умеет вернуть в SQL.
  const head=/^CREATE OR REPLACE (FUNCTION|PROCEDURE) (\S+)\((.*)\)$/.exec(signature[0]);
  if(!head)throw unsupported();
  const kind=head[1].toLowerCase(),name=routine.name;
  const names=head[2].split('.').map(readName);
  const args=head[3]?head[3].split(', '):[];
  if(names.length!==2 || names[0]!==schema || names[1]!==name || args.length!==selected.arguments.length)throw unsupported();
  const parameters=args.map((value,index)=>{
    const space=value.indexOf(' '),arg=selected.arguments[index];
    const argName=readName(value.slice(0,space)),type=readType(value.slice(space+1));
    if(!argName||argName!==arg.name||arg.mode!=='IN'||!type)throw unsupported();
    return {name:argName,type};
  });
  const returns=kind==='function'?/^ RETURNS (.+)$/.exec(signature[1]):null;
  const returnType=kind==='function'?readType(returns?.[1]):'integer',languageIndex=kind==='function'?2:1;
  if(!returnType || signature[languageIndex]!==' LANGUAGE plpgsql' || !/^AS \$[a-z_]*\$/i.test(signature[languageIndex+1]))throw unsupported();

  const lines=source.split('\n'),variables=[];
  if(lines[0]!=='' || lines[lines.length-1]!=='')throw unsupported();
  let cursor=1,count=0;
  if(lines[cursor]==='DECLARE'){
    cursor++;
    while(lines[cursor]!=='BEGIN'){
      const statement=/^    "(\w+)" ([^;]+?)(?: := (.+))?;$/.exec(lines[cursor++]);
      const type=readType(statement?.[2]);
      if(!type || variables.length>=100)throw unsupported();
      variables.push({name:statement[1],type,initial:statement[3]||''});
      lines[cursor-1]=`    "${statement[1]}" ${typeSql(type)}${statement[3]?' := '+statement[3]:''};`;
    }
  }
  function expect(line){if(lines[cursor++]!==line)throw unsupported();}
  function sequence(depth,end){
    if(depth>13)throw unsupported();
    const pad='    '.repeat(depth),blocks=[];
    if(lines[cursor]===pad+'NULL;'&&lines[cursor+1]===end){cursor++;return [];}
    while(lines[cursor]!==end){
      if(++count>500||!lines[cursor]?.startsWith(pad))throw unsupported();
      const statement=lines[cursor++].slice(pad.length);
      let match;
      if((match=/^"(\w+)" := (.+);$/.exec(statement))){
        blocks.push({...block('assign'),target:match[1],expression:match[2]});
      }else if((match=/^SELECT \* INTO "(\w+)"$/.exec(statement))){
        const query=[],indent=pad+'    ';
        expect(pad+'FROM (');
        while(lines[cursor]?.startsWith(indent))query.push(lines[cursor++].slice(indent.length));
        expect(pad+') AS selected_value;');
        blocks.push({...block('select'),target:match[1],query:query.join('\n')});
      }else if((match=/^IF (.+) THEN$/.exec(statement))){
        const then=sequence(depth+1,pad+'ELSE');expect(pad+'ELSE');
        const otherwise=sequence(depth+1,pad+'END IF;');expect(pad+'END IF;');
        blocks.push({...block('if'),condition:match[1],then,else:otherwise});
      }else if((match=/^WHILE (.+) LOOP$/.exec(statement))){
        const body=sequence(depth+1,pad+'END LOOP;');expect(pad+'END LOOP;');
        blocks.push({...block('while'),condition:match[1],body});
      }else if((match=/^FOR "(\w+)" IN \((.+)\)\.\.\((.+)\) LOOP$/.exec(statement))){
        const body=sequence(depth+1,pad+'END LOOP;');expect(pad+'END LOOP;');
        blocks.push({...block('for'),iterator:match[1],lower:match[2],upper:match[3],body});
      }else if((match=/^RETURN(?: (.+))?;$/.exec(statement))){
        blocks.push({...block('return'),expression:match[1]||''});
      }else throw unsupported();
    }
    return blocks;
  }
  expect('BEGIN');
  const blocks=sequence(1,'END;');expect('END;');expect('');
  const project={schemaVersion:3,kind,schema,name,returnType,parameters,variables,blocks,model:emptyModel()};
  // Проверяем, что из блоков получается то же тело.
  if(cursor!==lines.length || generate(project).body!==lines.join('\n'))throw unsupported();
  return project;
}
