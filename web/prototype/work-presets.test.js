import {strict as assert} from 'node:assert';
import * as crypto from 'node:crypto';
import {createWorkPresets,routineRevision} from './work-presets.js';

const data=new Map();
const storage={
  getItem:key=>data.get(key)??null,
  setItem:(key,value)=>data.set(key,value),
  removeItem:key=>data.delete(key),
};
const store=createWorkPresets(storage);
const profile={host:'127.0.0.1',port:5432,database:'demo',user:'owner'};
const target={...profile,schema:'public',name:'test',signature:'integer',kind:'Функция'};
const secret={password:'db-secret-value',connection_id:12,token:'api-secret-value',oid:25};
store.saveProfile({...profile,...secret});
store.saveProfile(profile);
assert.deepEqual(createWorkPresets(storage).profiles(),[profile]);
assert.equal(store.profiles()[0].password,undefined);
assert.throws(()=>store.saveProfile({...profile,port:0}));

store.remember({...target,...secret});
store.remember({...target,signature:'numeric'});
store.remember({...target,database:'other'});
store.remember({...target,user:'another_role'});
store.remember(target);
assert.deepEqual(store.recent(profile).map(item=>[item.signature,item.database]),
  [['integer','demo'],['integer','other'],['numeric','demo']]);
store.clearRecent(profile);
assert.deepEqual(store.recent(profile),[]);
assert.equal(store.recent({...profile,user:'another_role'}).length,1);
for(let index=0;index<14;index++)store.remember({...target,name:'f'+index});
assert.equal(store.recent(profile)[0].name,'f13');
assert.ok(store.recent(profile).length<=12);

const args=[
  {mode:'default',value:'ignored'},
  {mode:'null'},
  {mode:'value',value:''},
  {mode:'value',value:[null,'','a\tb\nc']},
  null,
];
const saved={name:'Smoke',target:{...target,...secret},args,points:[6,8,6],revision:'a'.repeat(64),...secret};
store.saveScenario(saved);
const [restored]=createWorkPresets(storage).scenarios(target);
assert.deepEqual(restored,{name:'Smoke',target,args:[{mode:'default'},...args.slice(1)],points:[6,8],revision:'a'.repeat(64)});
assert.deepEqual(store.scenarios({...target,signature:'numeric'}),[]);
assert.throws(()=>store.saveScenario({...saved,points:[0]}));
assert.throws(()=>store.saveScenario({...saved,args:[{mode:'value',value:'x'.repeat(65_537)}]}));
assert.throws(()=>store.saveScenario({...saved,args:Array.from({length:20},()=>({mode:'value',value:'x'.repeat(60_000)}))}));
assert.deepEqual(store.scenarios(target),[restored]);
store.saveScenario({...saved,points:[9]});
assert.deepEqual(store.scenarios(target)[0].points,[9]);
assert.equal(store.scenarios(target).length,1);
const serialized=[...data.values()].join('\n');
for(const value of ['db-secret-value','api-secret-value','connection_id','"oid"','ignored'])assert.ok(!serialized.includes(value));
store.removeScenario(restored);
store.removeProfile(profile);
assert.deepEqual(store.scenarios(target),[]);
assert.deepEqual(store.profiles(),[]);
data.set('pgdebug-work-v1-profiles','broken json');
assert.throws(()=>store.profiles(),/повреждён/);
assert.doesNotThrow(()=>store.scenarios(target));
const unavailable=createWorkPresets({...storage,getItem:()=>null,setItem:()=>{throw Error('Quota exceeded');}});
assert.throws(()=>unavailable.saveProfile(profile),/недоступно/);

// Node 14 запускает остальные тесты проекта, но Web Crypto в нём ещё нет.
if(!globalThis.crypto)globalThis.crypto=crypto.webcrypto||{
  subtle:{digest:async(_algorithm,data)=>crypto.createHash('sha256').update(data).digest()},
};
const type={name:'arg',mode:'IN',type:'custom',format:'composite',supported:true,fields:[{name:'a',type_name:'integer',type_oid:23}]};
const selected={definition:'CREATE FUNCTION body',arguments:[type],oid:100};
const revision=await routineRevision(selected);
assert.match(revision,/^[a-f0-9]{64}$/);
assert.equal(await routineRevision({...selected,oid:101,arguments:[{...type,fields:[{...type.fields[0],type_oid:203}]}]}),revision);
assert.notEqual(await routineRevision({...selected,definition:'CREATE FUNCTION new_body'}),revision);
assert.notEqual(await routineRevision({...selected,arguments:[{...type,fields:[{name:'b',type_name:'integer'}]}]}),revision);
console.log('PASS work presets: local profiles, durable object identity, argument modes, limits, deletion and schema revision.');
