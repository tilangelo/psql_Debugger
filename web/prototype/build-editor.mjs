import {build} from 'esbuild';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
const root=path.dirname(fileURLToPath(import.meta.url));
const output=path.join(root,'vendor');fs.mkdirSync(output,{recursive:true});
await build({absWorkingDir:root,entryPoints:['editor-src/code-editor.js'],bundle:true,format:'esm',platform:'browser',target:'es2020',minify:true,legalComments:'eof',outfile:'vendor/code-editor.js',metafile:true}).then(result=>{
  const packages=new Map();
  for(const name of Object.keys(result.metafile.inputs)){
    const match=name.match(/^(.*node_modules\/(?:@[^/]+\/)?[^/]+)\//);if(!match)continue;
    const folder=path.resolve(root,match[1]);const info=JSON.parse(fs.readFileSync(path.join(folder,'package.json'),'utf8'));
    const license=['LICENSE','LICENSE.txt','LICENSE.md'].map(file=>path.join(folder,file)).find(file=>fs.existsSync(file));
    if(!license)throw Error(`License not found: ${info.name}`);
    packages.set(info.name,`${info.name}@${info.version}\n${fs.readFileSync(license,'utf8').trim()}\n`);
  }
  const notices=[...packages.keys()].sort().map(name=>packages.get(name)).join('\n'+ '='.repeat(72)+'\n\n');
  fs.writeFileSync(path.join(output,'THIRD_PARTY_LICENSES.txt'),notices+'\n');
  const files=['editor-src/code-editor.js','build-editor.mjs','package.json','package-lock.json','vendor/code-editor.js','vendor/THIRD_PARTY_LICENSES.txt'];
  const hashes=Object.fromEntries(files.map(file=>[file,createHash('sha256').update(fs.readFileSync(path.join(root,file))).digest('hex')]));
  fs.writeFileSync(path.join(output,'build.json'),JSON.stringify(hashes,null,2)+'\n');
  const digest=JSON.stringify({packages:[...packages.keys()].sort(),bytes:fs.statSync(path.join(output,'code-editor.js')).size});
  console.log('Offline editor bundle built: '+digest);
});
