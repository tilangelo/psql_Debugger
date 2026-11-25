import assert from 'assert';
import {buildSelect, columnAllowed} from './select-builder.js';
const relation={schema:'Shop "A"',name:'Order Items',can_select_all:true};
const columns=[{number:1,name:'Order "Id"',type_oid:23},{number:2,name:'price',type_oid:23},{number:3,name:'enabled',type_oid:16}];
const options={relation,columns,mode:'value',columnNumber:2,filterNumber:1,parameter:'input_value',parameters:[{name:'input_value'}],routineName:'visual_price'};
const checks=[];
checks.push(['qualified identifiers and parameter',()=>{
  const query=buildSelect(options);
  assert(query.includes('"Shop ""A"""."Order Items"'));
  assert(query.includes('"__pgdbg_row"."Order ""Id""" = $1'));
  assert(query.startsWith('SELECT (\n    SELECT '));
  assert(!query.includes('LIMIT 1'));
  assert.equal(query,buildSelect({...options,routineName:'renamed_function'}));
}]);
checks.push(['aggregates and column permissions',()=>{
  assert(buildSelect({...options,mode:'count',columnNumber:null}).includes('pg_catalog.count(*)'));
  assert(buildSelect({...options,mode:'sum'}).includes('pg_catalog.sum("__pgdbg_row"."price")'));
  assert.throws(()=>buildSelect({...options,mode:'count',relation:{...relation,can_select_all:false}}));
  assert.throws(()=>buildSelect({...options,mode:'sum',columnNumber:3}));
  assert(!columnAllowed('min',columns[2]));
}]);
checks.push(['stale column, parameter and alias collision',()=>{
  assert.throws(()=>buildSelect({...options,columnNumber:99}));
  assert.throws(()=>buildSelect({...options,filterNumber:99}));
  assert.throws(()=>buildSelect({...options,parameter:'missing'}));
  assert(buildSelect({...options,routineName:'__pgdbg_row'}).includes('AS "__pgdbg_row_1"'));
  assert(buildSelect({...options,variables:[{name:'__pgdbg_row'}]}).includes('AS "__pgdbg_row_1"'));
}]);
checks.push(['no filter and no hidden modification',()=>{
  const before=JSON.stringify(options);
  assert(!buildSelect({...options,filterNumber:null}).includes('WHERE'));
  assert.equal(JSON.stringify(options),before);
  assert.throws(()=>buildSelect({...options,relation:{...relation,name:'bad\0name'}}));
}]);
for(const [name,test] of checks){test();console.log('PASS SELECT helper: '+name);}
