import {fitsAtDepth} from './model.js';
import assert from 'assert';
import {randomBytes} from 'crypto';
globalThis.crypto={randomUUID:()=>randomBytes(16).toString('hex')};
let checks=0;
function test(name,run){try{run();checks++;console.log('PASS '+name);}catch(error){process.exitCode=1;console.error('FAIL '+name,error);}}
import {example,block,generate,parseDraft,validate} from './model.js';
import {restoreGeneratedProject} from './reverse-project.js';
test('function emits definition, body and source mapping',()=>{const p=example(),r=generate(p);assert.deepEqual(r.errors,[]);assert.match(r.sql,/RETURNS integer/);assert.match(r.sql,/RETURN running_total;/);const n=p.blocks[0],map=r.sourceMap[n.id];assert.match(r.sql.split('\n')[map.start-1],/running_total.*:=/);assert.equal(map.bodyStart,5);});
test('procedure has no RETURNS and supports bare RETURN',()=>{const p=example('procedure');p.blocks.push(block('return'));const r=generate(p);assert.deepEqual(r.errors,[]);assert.match(r.sql,/CREATE OR REPLACE PROCEDURE/);assert.doesNotMatch(r.sql,/RETURNS/);assert.match(r.sql,/RETURN;/);});
test('both conditional branches can return; missing branch rejected',()=>{const p=example(),b=block('if');b.then.push(block('return'));b.else.push(block('return'));p.blocks=[b];assert.deepEqual(validate(p),[]);b.else=[];assert.match(validate(p).join(),/Добавьте RETURN/);});
test('JSON round trip preserves nested branch IDs and manual SQL',()=>{const p=example(),b=block('if');b.then.push(block('sql'));p.blocks.unshift(b);assert.deepEqual(parseDraft(JSON.stringify(p)),p);assert.match(generate(p).sql,/IF input_value > 0 THEN/);});
test('malformed drafts fail before UI consumes them',()=>{for(const v of [null,{}, {...example(),blocks:null},{...example(),parameters:null}])assert.throws(()=>parseDraft(JSON.stringify(v)));const p=example();p.blocks.push({...p.blocks[0]});assert.match(validate(p).join(),/повторяющийся/);});
test('identifier and target errors prevent generation',()=>{const p=example();p.name='test; DROP SCHEMA public';assert.equal(generate(p).sql,'');p.name='test';p.blocks[0].target='missing';assert.match(validate(p).join(),/объявленную/);});
test('manual SQL dollar tag cannot terminate generated body',()=>{const p=example('procedure');p.blocks=[{...block('sql'),sql:"RAISE NOTICE '$body$';"}];const r=generate(p);assert.deepEqual(r.errors,[]);assert.match(r.sql,/AS \$body_\$/);});
test('SELECT INTO uses scalar subquery and declared target',()=>{const p=example();p.blocks.unshift(block('select'));assert.match(generate(p).sql,/SELECT \* INTO "running_total"\n    FROM \(/);});
test('nested WHILE and FOR preserve block IDs and map individual statements',()=>{
  const p=example(),step={...block('assign'),expression:'running_total + i'};
  const loop={...block('for'),upper:'3',body:[step]};
  p.blocks=[{...block('while'),condition:'running_total < 10',body:[loop]},block('return')];
  const generated=generate(p),lines=generated.body.split('\n');
  assert.equal(lines[generated.sourceMap[step.id].bodyStart-1],'            "running_total" := running_total + i;');
  assert.equal(lines[generated.sourceMap[loop.id].bodyEnd-1],'        END LOOP;');
  assert.deepEqual(parseDraft(JSON.stringify(p)),p);
  loop.upper='';
  assert.match(validate(p).join(),/границы цикла FOR/);
  p.blocks=[{...block('while'),condition:'true',body:[block('return')]}];
  assert.match(validate(p).join(),/Добавьте RETURN/);
});
test('constructor restores supported catalog definitions and checks the original body',()=>{
  const p=example(),query={...block('select'),query:'SELECT 1\nWHERE true'};
  p.blocks=[
    {...block('for'),body:[{...block('if'),then:[query]}]},
    {...block('while'),condition:'running_total < 3',body:[block('assign')]},
    block('return'),
  ];
  const definition='CREATE OR REPLACE FUNCTION public.visual_smoke(input_value integer)\n RETURNS integer\n LANGUAGE plpgsql\nAS $function$';
  const body=generate(p).body;
  const catalog={schema:'public',routines:[{oid:1,name:p.name}],selected:{oid:1,source:body.replace('"running_total" integer','"running_total" pg_catalog.int4'),definition,arguments:[{name:'input_value',mode:'IN'}]}};
  const restored=restoreGeneratedProject(catalog);
  assert.equal(generate(restored).body,body);
  assert.equal(restored.blocks[0].body[0].then[0].query,query.query);
  assert.deepEqual(restored.blocks[0].body[0].else,[]);
  assert.throws(()=>restoreGeneratedProject({...catalog,selected:{...catalog.selected,definition:definition.replace('AS $function$', ' SECURITY DEFINER\nAS $function$')}}));
  assert.throws(()=>restoreGeneratedProject({...catalog,selected:{...catalog.selected,source:body.replace('RETURN running_total;', 'RAISE NOTICE \'done\';')}}));
});
test('bare RETURN from a generated procedure restores without an expression',()=>{
  const p=example('procedure');p.blocks=[block('return')];
  const restored=restoreGeneratedProject({schema:'public',routines:[{oid:1,name:p.name}],selected:{
    oid:1,source:generate(p).body,definition:'CREATE OR REPLACE PROCEDURE public.visual_smoke_proc(input_value integer)\n LANGUAGE plpgsql\nAS $procedure$',
    arguments:[{name:'input_value',mode:'IN'}],
  }});
  assert.equal(generate(restored).body,generate(p).body);
});

// На последнем уровне помещается оператор без вложенных веток.
assert.equal(fitsAtDepth({type:'return'},12),true);
assert.equal(fitsAtDepth({type:'if',then:[],else:[]},12),false);
assert.equal(fitsAtDepth({type:'if',then:[{type:'return'}],else:[]},11),true);
assert.equal(fitsAtDepth({type:'while',body:[{type:'if',then:[],else:[]}]},11),false);
