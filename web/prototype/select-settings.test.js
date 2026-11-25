import assert from 'assert';
import {randomBytes} from 'crypto';
globalThis.crypto={randomUUID:()=>randomBytes(16).toString('hex')};
import {example,parseDraft,generate} from './model.js';
import {validateSettings,findSavedRelation,findSavedLink,resolveSavedField} from './select-settings.js';
const saved={schema:'sales',table:'orders',mode:'value',column:{side:'joined',name:'name'},filter:{side:'source',name:'id',parameter:'input_value'},join:{constraint:'customer_link',kind:'left',referencedSchema:'sales',referencedTable:'customers',columns:['customer_id'],referencedColumns:['id']}};
const p=example();p.blocks[0]={id:'select',type:'select',query:'SELECT 123 /* ручной текст */',target:'running_total',assistant:saved};
const restored=parseDraft(JSON.stringify(p));assert.deepEqual(restored,p);assert(generate(restored).sql.includes('SELECT 123 /* ручной текст */'));
console.log('PASS SELECT settings: complete round trip preserves independent SQL');
const metadata={relations:[{oid:999,schema:'sales',name:'orders'}]};assert.equal(findSavedRelation(saved,metadata).oid,999);
const key={name:'customer_link',referenced_oid:111,referenced_schema:'sales',referenced_table:'customers',columns:['customer_id'],referenced_columns:['id']};
assert.equal(findSavedLink(saved.join,{foreign_keys:[key]}),key);
assert.equal(resolveSavedField(saved.column,{columns:[]},{details:{columns:[{name:'name',number:8}]}}),'joined:8');
console.log('PASS SELECT settings: fresh OIDs and column positions resolved by names');
assert.throws(()=>findSavedRelation(saved,{relations:[]}));assert.throws(()=>findSavedLink(saved.join,{foreign_keys:[{...key,referenced_columns:['other']}]}));assert.throws(()=>resolveSavedField(saved.column,null,{details:{columns:[]}}));
assert.throws(()=>validateSettings({...saved,oid:99}));assert.throws(()=>validateSettings({...saved,mode:'delete'}));
console.log('PASS SELECT settings: missing objects and changed keys rejected');
for(const version of [1,2]){const old=example();old.schemaVersion=version;if(version===1)delete old.model;const next=parseDraft(JSON.stringify(old));assert.equal(next.schemaVersion,3);assert.equal(generate(old).sql,generate(next).sql);}
console.log('PASS SELECT settings: versions 1 and 2 migrate without changing SQL');
