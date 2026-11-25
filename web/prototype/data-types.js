export const constructorTypes=['integer','text','boolean','bigint','numeric','date','timestamp','uuid',
  'smallint','real','double precision','varchar','json','jsonb','timestamptz','time','interval','bytea'];
export const typeByOid=new Map([[16,'boolean'],[20,'bigint'],[21,'integer'],[23,'integer'],[25,'text'],[1042,'text'],[1043,'text'],[1700,'numeric'],[1082,'date'],[1114,'timestamp'],[2950,'uuid'],[700,'real'],[701,'double precision'],[114,'json'],[3802,'jsonb'],[1184,'timestamptz'],[1083,'time'],[1186,'interval'],[17,'bytea']]);
const builtinNames={integer:'int4',boolean:'bool',bigint:'int8',smallint:'int2',real:'float4','double precision':'float8'};
export const builtinArrayType=type=>({schema:'pg_catalog',name:builtinNames[type]||type,array:true});
const typeIdentifier=value=>typeof value==='string'&&!!value&&new TextEncoder().encode(value).length<=63&&!/[\x00-\x1f\x7f-\x9f]/.test(value);
export const namedType=value=>value&&typeof value==='object'&&!Array.isArray(value)&&Object.keys(value).every(k=>['schema','name','array'].includes(k))
  &&typeIdentifier(value.schema)&&typeIdentifier(value.name)&&(value.array===undefined||typeof value.array==='boolean');
export const knownType=value=>constructorTypes.includes(value)||namedType(value);
const quoteTypePart=value=>'"'+value.replace(/"/g,'""')+'"';
export function typeSql(value){
  return typeof value==='string'?value:`${quoteTypePart(value.schema)}.${quoteTypePart(value.name)}${value.array?'[]':''}`;
}
const typeKey=value=>namedType(value)?JSON.stringify({schema:value.schema,name:value.name,...(value.array?{array:true}:{})}):value||'';
export function readTypeChoice(value){
  if(!value.startsWith('{'))return value;
  try{const parsed=JSON.parse(value);return namedType(parsed)?parsed:value;}catch{return value;}
}
export function fillTypeSelect(select,value,types=[]){
  select.replaceChildren();
  const simple=document.createElement('optgroup');simple.label='Встроенные типы';
  const arrays=document.createElement('optgroup');arrays.label='Встроенные массивы';
  const custom=document.createElement('optgroup');custom.label='Типы из базы';
  for(const item of constructorTypes){
    simple.append(new Option(item==='timestamp'?'timestamp (без часового пояса)':item,item));
    arrays.append(new Option(item+'[]',typeKey(builtinArrayType(item))));
  }
  for(const item of types)custom.append(new Option(item.schema+'.'+item.name+(item.array?'[]':''),typeKey(item)));
  const key=typeKey(value);
  if(![...simple.children,...arrays.children,...custom.children].some(option=>option.value===key)){
    const option=new Option(namedType(value)?typeSql(value):value||'Выберите тип',key);
    select.append(option);
  }
  select.append(simple,arrays);
  if(custom.children.length)select.append(custom);
  select.value=key;
}
const identifier=value=>typeof value==='string'&&/^[a-z_][a-z_0-9]{0,62}$/i.test(value);

// Показываем ошибки у полей и оставляем введённый текст.
export function declarationIssues(project){
  const issues=[],names=new Map();
  for(const key of ['parameters','variables'])for(const [index,item] of (Array.isArray(project[key])?project[key]:[]).entries()){
    const issue=(field,message)=>issues.push({key,index,field,message});
    if(!identifier(item?.name))issue('name',!item?.name?'Введите имя.':'Имя: до 63 латинских букв, цифр и _, первый символ - буква или _.');
    if(item?.name){const previous=names.get(item.name);if(previous){issue('name','Имя уже используется в параметрах или переменных.');issues.push({...previous,field:'name',message:'Имя уже используется в параметрах или переменных.'});}else names.set(item.name,{key,index});}
    if(!knownType(item?.type))issue('type','Выберите поддерживаемый тип.');
    if(item?.initial!==undefined&&(typeof item.initial!=='string'||item.initial.includes('\0')))issue('initial','Начальное выражение должно быть текстом без нулевого символа.');
  }
  return issues;
}

export function argumentHint(oid){return ({20:'9223372036854775807',1700:'123.4567890123456789',1082:'2026-11-18',1114:'2026-11-18 14:30:00.123456',2950:'123e4567-e89b-12d3-a456-426614174000',16:'true или false'})[oid]||'Значение';}
export function callArgumentHint(arg){
  return ({
    json:'{"ключ": "значение"}',timestamptz:'2026-11-20T14:30:00+10:00',
    time:'14:30:00',interval:'2 days 03:30:00',hex:'\\x48656c6c6f',
    array:'{элемент1,элемент2}',
  })[arg.format]||argumentHint(arg.type_oid);
}
export function callArgumentError(arg,input){
  const error=argumentError(arg.type_oid,input);
  if(error||input===null)return error;
  if(arg.format==='json'){
    try{JSON.parse(input);}catch{return 'Введите корректный JSON.';}
  }
  return '';
}

// В записи PostgreSQL кавычки отличают пустой текст от NULL.
export function recordLiteral(values){
  return '('+values.map(value=>value===null?'':'"'+value.replace(/[\\"]/g,'\\$&')+'"').join(',')+')';
}
function dateValid(text){
  const match=/^(\d{4})-(\d{2})-(\d{2})$/.exec(text);if(!match)return false;
  const [,y,m,d]=match.map(Number),leap=y%4===0&&(y%100!==0||y%400===0);
  return y>0&&m>=1&&m<=12&&d>=1&&d<=[31,leap?29:28,31,30,31,30,31,31,30,31,30,31][m-1];
}
export function argumentError(oid,input){
  if(input===null)return '';
  if(typeof input!=='string'||input.includes('\0'))return 'Введите текст без нулевого символа.';
  const value=input.trim();
  if([20,21,23].includes(oid)){
    if(!/^[+-]?\d+$/.test(value))return 'Введите целое число.';
    const n=BigInt(value),bits=oid===20?64n:oid===21?16n:32n,limit=1n<<(bits-1n);
    if(n< -limit||n>=limit)return 'Число выходит за диапазон '+(oid===20?'bigint':oid===21?'smallint':'integer')+'.';
  }else if(oid===1700&&!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?$/.test(value))return 'Введите конечное десятичное число с точкой; допустима запись 1e-8.';
  else if(oid===1082&&!dateValid(value))return 'Введите существующую дату ГГГГ-ММ-ДД, годы 0001-9999.';
  else if(oid===1114){const m=/^(\d{4}-\d{2}-\d{2})[ T](\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,6}))?$/.exec(value);if(!m||!dateValid(m[1])||+m[2]>23||+m[3]>59||+m[4]>59)return 'Введите ГГГГ-ММ-ДД ЧЧ:ММ:СС, до 6 цифр после точки, без часового пояса.';}
  else if(oid===2950&&!/^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(value))return 'Введите UUID вида '+argumentHint(oid)+'.';
  else if(oid===16&&!/^(true|false)$/i.test(value))return 'Введите true или false.';
  else if([700,701].includes(oid)&&(!value||!Number.isFinite(Number(value))))return 'Введите конечное число.';
  return '';
}
