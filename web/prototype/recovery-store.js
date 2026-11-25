const bytes=value=>new TextEncoder().encode(value).length;
export class RecoveryConflict extends Error {}
export function createRecoveryStore(storage,key,kind,validate,{writer,now=()=>Date.now()}={}){
  let expected;
  function raw(){try{return storage.getItem(key);}catch{throw Error('Хранилище браузера недоступно. Сохраните нужный текст в файл.');}}
  function parse(value){
    if(value===null)return null;
    if(bytes(value)>2_200_000)throw Error('Автокопия превышает допустимый размер.');
    let data;try{data=JSON.parse(value);}catch{throw Error('Автокопия повреждена: неверный JSON.');}
    if(!data||typeof data!=='object'||Array.isArray(data)||Object.keys(data).some(k=>!['version','kind','at','writer','value'].includes(k))||data.version!==1||data.kind!==kind||!Number.isSafeInteger(data.at)||data.at<0||typeof data.writer!=='string'||data.writer.length>256)throw Error('Автокопия имеет неизвестный формат.');
    return {...data,value:validate(data.value)};
  }
  function compare(){const value=raw();if(value!==expected)throw new RecoveryConflict('Автокопия изменена в другой вкладке. Текущий текст оставлен в рабочей области; восстановите или удалите найденную копию явно.');return value;}
  return {
    read(){expected=raw();return parse(expected);},
    write(value){compare();const data={version:1,kind,at:now(),writer,value:validate(value)};const json=JSON.stringify(data);if(bytes(json)>2_200_000)throw Error('Автокопия превышает допустимый размер.');try{storage.setItem(key,json);}catch{throw Error('Не удалось записать автокопию: хранилище заполнено или запрещено. Сохраните работу в файл.');}expected=json;return data;},
    clear(){compare();try{storage.removeItem(key);}catch{throw Error('Не удалось удалить автокопию.');}expected=null;},
    unchanged(){return raw()===expected;},
  };
}
