import assert from 'assert';
import {diagnosticReport} from './diagnostic-report.js';
const input={version:'0.6.0',catalog:{server_version:'17.11',database:'test',current_user:'reader',password:'never-export',token:'never-export',debugger:'Доступен',schema:'public',routines:[{oid:1,name:'test_fn'}],selected:{oid:1,arguments:[{name:'value',mode:'IN',type:'text',value:'sensitive-value'}],definition:'SECRET FUNCTION SOURCE'}},session:{phase:'finished',values:['sensitive-value'],outcome:{call:'failed',rollback:'confirmed',sqlstate:'P0001',text:'private-error-details'}},failure:{path:'/api/prepare',status:400,message:'private-request-details'}};
const basic=diagnosticReport(input);assert(basic.includes('P0001'));assert(basic.includes('reader'));for(const value of ['sensitive-value','never-export','SECRET FUNCTION SOURCE','private-error-details','private-request-details'])assert(!basic.includes(value));
console.log('PASS diagnostics: default report omits credentials, source and argument values');
const full=diagnosticReport({...input,includeDetails:true});assert(full.includes('SECRET FUNCTION SOURCE'));assert(full.includes('private-error-details'));assert(!full.includes('never-export'));assert(diagnosticReport({}).includes('не выбрана'));
console.log('PASS diagnostics: source/details require explicit choice; disconnected report works');
