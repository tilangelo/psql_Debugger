// Одинаковый снимок структуры используется конструктором и текстовыми подсказками.
export function createMetadataReader(request,now=()=>Date.now()){
  const cache=new Map(),inflight=new Map();let generation=0;
  return {
    clear(){generation++;cache.clear();inflight.clear();},
    async read(body,alive=()=>true,force=false){
      const key=JSON.stringify(body),saved=cache.get(key),epoch=generation;
      if(!force&&saved&&now()-saved.at<30000)return saved.result.metadata;
      let pending=inflight.get(key);
      if(!pending){pending=request('/api/metadata',body);inflight.set(key,pending);}
      let result;
      try{result=await pending;}finally{if(inflight.get(key)===pending)inflight.delete(key);}
      if(epoch===generation&&alive()){if(cache.size>=100)cache.clear();cache.set(key,{at:now(),result});}
      return result.metadata;
    },
  };
}
