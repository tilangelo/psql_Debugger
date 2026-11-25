import {requestDatabase} from './database-client.js';
import {createMetadataReader} from './metadata-reader.js';
const reader=createMetadataReader(requestDatabase);
export let metadataContext=null;
export const readMetadata=(...args)=>reader.read(...args);
export function clearMetadataCache(){reader.clear();}
document.addEventListener('catalogchange',event=>{
  const result=event.detail;
  const next=result?{connection_id:result.connection_id,database:result.database,schema:result.schema}:null;
  if(JSON.stringify(next)!==JSON.stringify(metadataContext)){
    metadataContext=next;reader.clear();
    document.dispatchEvent(new CustomEvent('metadata-context-change'));
  }
});
