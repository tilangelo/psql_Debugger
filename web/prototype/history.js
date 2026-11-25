const bytes=value=>new TextEncoder().encode(value).length;
// Ввод в одном поле объединяем в один шаг отмены.
export function createHistory(initial,{limit=50,maxBytes=8_000_000,now=()=>Date.now()}={}){
  let entries=[],index=-1,total=0;
  function trim(){while(entries.length>1&&(entries.length>limit||total>maxBytes)){total-=entries.shift().bytes;index--;}}
  function record(value,{group=null}={}){
    const json=JSON.stringify(value);if(entries[index]?.json===json)return false;
    const size=bytes(json);if(size>maxBytes)throw Error('Слишком большой черновик для истории действий.');
    for(const removed of entries.splice(index+1))total-=removed.bytes;
    const entry={json,bytes:size,group,at:now()};
    if(group&&index>0&&entries[index].group===group&&entry.at-entries[index].at<600){total-=entries[index].bytes;entries[index]=entry;}
    else{entries.push(entry);index++;}
    total+=size;trim();return true;
  }
  record(initial);
  return {
    record,
    undo(){if(index<=0)return null;entries[index].group=null;return JSON.parse(entries[--index].json);},
    redo(){if(index>=entries.length-1)return null;entries[index].group=null;return JSON.parse(entries[++index].json);},
    available:()=>({undo:index>0,redo:index<entries.length-1}),
    breakGroup(){if(entries[index])entries[index].group=null;},
    current:()=>JSON.parse(entries[index].json),
  };
}
