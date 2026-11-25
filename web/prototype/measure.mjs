// Из папки web/prototype: node measure.mjs [--write-project]. Ничего не отправляет в PostgreSQL.
import {performance} from 'node:perf_hooks';
import {writeFileSync} from 'node:fs';
import {generate,parseDraft} from './model.js';
const project={schemaVersion:3,model:{name:'',tables:[]},kind:'function',schema:'public',name:'benchmark_project',returnType:'integer',parameters:Array.from({length:100},(_,i)=>({name:'p'+i,type:'integer'})),variables:Array.from({length:100},(_,i)=>({name:'v'+i,type:'integer',initial:'0'})),blocks:[...Array.from({length:499},(_,i)=>({id:'b'+i,type:'assign',target:'v0',expression:'v0 + 1'})),{id:'r',type:'return',expression:'v0'}]};
const encoded=JSON.stringify(project),samples=[];
for(let i=0;i<100;i++){const start=performance.now();const parsed=parseDraft(encoded),result=generate(parsed);if(result.errors.length)throw Error(result.errors.join('\n'));samples.push(performance.now()-start);}
samples.sort((a,b)=>a-b);
console.log(JSON.stringify({blocks:500,parameters:100,variables:100,bytes:Buffer.byteLength(encoded),iterations:100,parse_generate_median_ms:samples[50],parse_generate_p95_ms:samples[95]},null,2));
if(process.argv.includes('--write-project')){writeFileSync('benchmark.pgflow.json',encoded+'\n');console.log('Создан benchmark.pgflow.json для проверки интерфейса.');}
