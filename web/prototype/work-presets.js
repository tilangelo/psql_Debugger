const prefix='pgdebug-work-v1-';
const limits={profiles:20,recent:12,scenarios:30};
const connectionFields=['host','port','database','user'];
const routineFields=[...connectionFields,'schema','name','signature','kind'];
const text=(value,limit)=>typeof value==='string'&&value.length<=limit;
const integer=(value,min,max)=>Number.isInteger(value)&&value>=min&&value<=max;
const pick=(value,keys)=>Object.fromEntries(keys.map(key=>[key,value[key]]));
const requiredText=(value,limit)=>text(value,limit)&&value.length>0;
const endpointFields=['host','port','user'];
const same=(left,right,keys)=>keys.every(key=>left[key]===right[key]);

function connection(value){
  if(!value||!integer(value.port,1,65535)||!requiredText(value.host,255)||
    !requiredText(value.user,255)||!requiredText(value.database,255))return null;
  return pick(value,connectionFields);
}

function routine(value){
  if(!connection(value)||!requiredText(value.schema,255)||!requiredText(value.name,255)||
    !text(value.signature,16_384)||!requiredText(value.kind,80))return null;
  return pick(value,routineFields);
}

function argument(value){
  if(value===null)return null; // OUT
  if(value?.mode==='null'||value?.mode==='default')return {mode:value.mode};
  if(value?.mode==='value'){
    const input=value.value;
    if(text(input,65_536))return {mode:'value',value:input};
    if(Array.isArray(input)&&input.length<=1600&&input.every(field=>field===null||text(field,65_536))){
      return {mode:'value',value:input.slice()};
    }
  }
  throw Error('Сценарий содержит недопустимый аргумент.');
}

function scenario(value){
  const target=routine(value?.target);
  if(!target||!requiredText(value.name,80)||!text(value.revision,64)||!/^[a-f0-9]{64}$/.test(value.revision)||
    !Array.isArray(value.args)||value.args.length>100||!Array.isArray(value.points)||value.points.length>4096||
    !value.points.every(line=>integer(line,1,2_147_483_647)))return null;
  return {target,name:value.name,revision:value.revision,args:value.args.map(argument),points:[...new Set(value.points)]};
}

const normalize={profiles:connection,recent:routine,scenarios:scenario};
const byteSize=value=>new TextEncoder().encode(value).length;

export function createWorkPresets(storage){
  function read(kind){
    const raw=storage.getItem(prefix+kind);
    if(raw===null)return [];
    try{
      if(raw.length>1_000_000)throw Error();
      const entries=JSON.parse(raw);
      if(!Array.isArray(entries)||entries.length>limits[kind])throw Error();
      return entries.map(entry=>{
        const valid=normalize[kind](entry);
        if(!valid)throw Error();
        return valid;
      });
    }catch{
      throw Error('Сохранённый список повреждён.');
    }
  }
  function write(kind,entries){
    if(!entries.length){storage.removeItem(prefix+kind);return;}
    const output=JSON.stringify(entries);
    if(byteSize(output)>1_000_000)throw Error('Сохранённые данные слишком велики. Удалите старые сценарии или сократите аргументы.');
    try{storage.setItem(prefix+kind,output);}
    catch{throw Error('Хранилище браузера заполнено или недоступно.');}
  }
  function keep(kind,value,match){
    const entry=normalize[kind](value);
    if(!entry)throw Error('Не удалось сохранить: заполните данные.');
    const entries=read(kind).filter(item=>!match(item,entry));
    if(entries.length===limits[kind]&&kind!=='recent')throw Error('Список заполнен. Сначала удалите одну из сохранённых записей.');
    write(kind,[entry,...entries].slice(0,limits[kind]));
  }
  const sameProfile=(a,b)=>same(a,b,connectionFields);
  const sameRoutine=(a,b)=>same(a,b,routineFields);
  const sameScenario=(a,b)=>a.name===b.name&&sameRoutine(a.target,b.target);
  return {
    profiles:()=>read('profiles'),
    saveProfile:value=>keep('profiles',value,sameProfile),
    removeProfile:value=>write('profiles',read('profiles').filter(item=>!sameProfile(item,value))),
    recent:endpoint=>read('recent').filter(item=>same(item,endpoint,endpointFields)),
    remember:value=>keep('recent',value,sameRoutine),
    clearRecent:endpoint=>write('recent',read('recent').filter(item=>!same(item,endpoint,endpointFields))),
    scenarios:target=>read('scenarios').filter(item=>sameRoutine(item.target,target)),
    saveScenario:value=>keep('scenarios',value,sameScenario),
    removeScenario:value=>write('scenarios',read('scenarios').filter(item=>!sameScenario(item,value))),
  };
}

export const workPresets=createWorkPresets({
  getItem:key=>localStorage.getItem(key),
  setItem:(key,value)=>localStorage.setItem(key,value),
  removeItem:key=>localStorage.removeItem(key),
});

export async function routineRevision(selected){
  // Типы и имена полей сохраняют смысл после пересоздания базы, в отличие от OID.
  const args=selected.arguments.map(arg=>[
    arg.mode,arg.name,arg.type,arg.format,arg.supported,arg.has_default,arg.enum_labels,
    arg.fields?.map(field=>[field.name,field.type_name]),
  ]);
  const data=new TextEncoder().encode(JSON.stringify([selected.definition,args]));
  const hash=await crypto.subtle.digest('SHA-256',data);
  return Array.from(new Uint8Array(hash),byte=>byte.toString(16).padStart(2,'0')).join('');
}
