import assert from 'assert';
import fs from 'fs';
import {buildSelect} from './select-builder.js';
const source={oid:1,schema:'join_checks',name:'child',can_select_all:true};
const parent={oid:2,schema:'join_checks',name:'parent',can_select_all:true};
const columns=[{number:1,name:'id',type_oid:23},{number:2,name:'tenant',type_oid:23},{number:3,name:'parent_key',type_oid:23},{number:4,name:'amount',type_oid:23}];
const joinedColumns=[{number:1,name:'tenant',type_oid:23},{number:2,name:'key',type_oid:23},{number:3,name:'label',type_oid:25},{number:4,name:'region_id',type_oid:23}];
const foreignKey={name:'parent_link',columns:['tenant','parent_key'],referenced_oid:2,referenced_schema:'join_checks',referenced_table:'parent',referenced_columns:['tenant','key']};
const join={kind:'inner',relation:parent,columns:joinedColumns,foreignKey};
const options={relation:source,columns,mode:'value',columnNumber:3,columnSide:'joined',filterNumber:1,parameter:'id',parameters:[{name:'id'}],variables:[],routineName:'join_result',join};
const query=buildSelect(options);
assert(query.includes('INNER JOIN "join_checks"."parent"'));
assert(query.includes('"__pgdbg_row"."tenant" = "__pgdbg_join"."tenant" AND "__pgdbg_row"."parent_key" = "__pgdbg_join"."key"'));
assert(query.includes('SELECT "__pgdbg_join"."label"'));
assert(buildSelect({...options,join:{...join,kind:'left'}}).includes('LEFT JOIN'));
console.log('PASS JOIN: composite key order and explicit INNER/LEFT');
const selfRelation={oid:3,schema:'join_checks',name:'staff',can_select_all:true};
const selfColumns=[{number:1,name:'id',type_oid:23},{number:2,name:'manager_id',type_oid:23},{number:3,name:'name',type_oid:25}];
const selfJoin={kind:'left',relation:selfRelation,columns:selfColumns,foreignKey:{columns:['manager_id'],referenced_oid:3,referenced_schema:'join_checks',referenced_table:'staff',referenced_columns:['id']}};
const self=buildSelect({...options,relation:selfRelation,columns:selfColumns,join:selfJoin});
assert(self.includes('"__pgdbg_row"."manager_id" = "__pgdbg_join"."id"'));
assert(buildSelect({...options,variables:[{name:'__pgdbg_join'}]}).includes('AS "__pgdbg_join_1"'));
assert(buildSelect({...options,filterSide:'joined',filterNumber:1}).includes('WHERE "__pgdbg_join"."tenant" = $1'));
console.log('PASS JOIN: self relation, filter sides and alias collision');
assert.throws(()=>buildSelect({...options,join:{...join,relation:{...parent,oid:99}}}));
assert.throws(()=>buildSelect({...options,join:{...join,columns:[]}}));
assert.throws(()=>buildSelect({...options,join:{...join,foreignKey:{...foreignKey,referenced_columns:['key']}}}));
assert.throws(()=>buildSelect({...options,join:{...join,kind:'cross'}}));
assert.throws(()=>buildSelect({...options,mode:'count',join:{...join,relation:{...parent,can_select_all:false}}}));
assert.throws(()=>buildSelect({...options,join:null}));
console.log('PASS JOIN: stale metadata, missing privileges and unknown modes rejected');
const region={oid:4,schema:'join_checks',name:'region',can_select_all:true};
const regionColumns=[{number:1,name:'tenant',type_oid:23},{number:2,name:'label',type_oid:25}];
const nextJoin={from:'joined',kind:'left',relation:region,columns:regionColumns,foreignKey:{
  name:'region_link',columns:['region_id'],referenced_oid:4,referenced_schema:'join_checks',referenced_table:'region',referenced_columns:['tenant'],
}};
const chain=buildSelect({...options,columnSide:'join_2',columnNumber:2,extra:[nextJoin]});
assert(chain.includes('LEFT JOIN "join_checks"."region"'));
assert(chain.includes('"__pgdbg_join"."region_id" = "__pgdbg_join_2"."tenant"'));
assert(chain.includes('SELECT "__pgdbg_join_2"."label"'));
assert.throws(()=>buildSelect({...options,extra:[{...nextJoin,from:'join_2'}]}));
assert.throws(()=>buildSelect({...options,join:null,extra:[nextJoin]}));
console.log('PASS chained JOIN: origin, result and forward references checked');
const before=JSON.stringify(options);buildSelect(options);assert.equal(JSON.stringify(options),before);
const fixtures=[
  {name:'inner_value',query,returnType:'text',argument:11,expected:'A'},
  {name:'composite_value',query,returnType:'text',argument:12,expected:'B'},
  {name:'left_null',query:buildSelect({...options,join:{...join,kind:'left'}}),returnType:'text',argument:13,expected:null},
  {name:'inner_count',query:buildSelect({...options,mode:'count',filterNumber:null}),returnType:'integer',argument:0,expected:'2'},
  {name:'left_count',query:buildSelect({...options,mode:'count',filterNumber:null,join:{...join,kind:'left'}}),returnType:'integer',argument:0,expected:'3'},
  {name:'left_sum',query:buildSelect({...options,mode:'sum',columnSide:'source',columnNumber:4,filterNumber:null,join:{...join,kind:'left'}}),returnType:'integer',argument:0,expected:'60'},
  {name:'self_value',query:self,returnType:'text',argument:2,expected:'Boss'},
  {name:'many_rows',query:buildSelect({...options,filterNumber:null}),returnType:'text',argument:0,expectedError:'21000'},
  {name:'chain_value',query:chain,returnType:'text',argument:11,expected:'North'},
  {name:'chain_null',query:chain,returnType:'text',argument:12,expected:null},
];
const path=new URL('../server/tests/join_queries.json',import.meta.url),encoded=JSON.stringify(fixtures,null,2)+'\n';
if(process.argv.includes('--write-fixtures'))fs.writeFileSync(path,encoded);
else assert.equal(fs.readFileSync(path,'utf8'),encoded,'Regenerate SQL integration fixtures with node web/prototype/join.test.js --write-fixtures');
console.log('PASS JOIN: exact generated queries shared with real PostgreSQL tests');
