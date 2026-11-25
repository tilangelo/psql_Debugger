import {typeByOid} from './data-types.js';
import {parseDraft,generate} from './model.js';
import {buildSelect} from './select-builder.js';
import {validateSettings,findSavedLink,joinSides} from './select-settings.js';
const quote=name=>'"'+name.replace(/"/g,'""')+'"';
function dataType(column){
  const type=typeByOid.get(column.type_oid);if(type)return type;
  throw Error(`Тип ${column.data_type||column.type_oid} пока не поддерживается шаблонами.`);
}
export function createTemplate(project,settings,context,kind,makeId=()=>crypto.randomUUID()){
  validateSettings(settings);
  if(!['lookup','sum','status'].includes(kind))throw Error('Неизвестный шаблон.');
  const {relation,details,joined,extra=[]}=context;
  if(!relation || !details || relation.schema!==settings.schema || relation.name!==settings.table)throw Error('Выберите доступную основную таблицу.');
  const tables=[{relation,details},joined,...extra];
  const selectField=field=>{
    const column=tables[joinSides.indexOf(field?.side)]?.details.columns.find(c=>c.name===field?.name);
    if(!column)throw Error('Выберите доступное поле.');return column;
  };
  const column=selectField(settings.column),filter=settings.filter?selectField(settings.filter):null;
  if(kind!=='sum' && !filter)throw Error('Для этого шаблона выберите поле фильтра.');
  const next=parseDraft(JSON.stringify(project));next.schemaVersion=3;
  const inputName=settings.filter?.parameter||'input_value';
  next.parameters=filter?[{name:inputName,type:dataType(filter)}]:[];
  const used=new Set(next.parameters.map(p=>p.name));
  function unique(name){while(used.has(name))name+='_1';used.add(name);return name;}
  if(kind==='status'){
    if(settings.join || settings.column.side!=='source' || settings.filter.side!=='source')throw Error('Для изменения поля выберите основную таблицу без JOIN.');
    if(!['r','p'].includes(relation.kind) || column.can_update!==true)throw Error('Для изменения поля нужна обычная таблица и право UPDATE на выбранную колонку.');
    const newValue=unique('new_value'),changed=unique('changed_rows');
    next.kind='procedure';next.parameters.push({name:newValue,type:dataType(column)});next.variables=[{name:changed,type:'integer',initial:''}];
    next.blocks=[
      {id:makeId(),type:'sql',sql:`UPDATE ${quote(relation.schema)}.${quote(relation.name)}\nSET ${quote(column.name)} = $2\nWHERE ${quote(filter.name)} = $1;`},
      {id:makeId(),type:'sql',sql:`GET DIAGNOSTICS ${quote(changed)} = ROW_COUNT;`},
      {id:makeId(),type:'if',condition:`${quote(changed)} = 0`,then:[{id:makeId(),type:'sql',sql:"RAISE EXCEPTION 'Запись не найдена' USING ERRCODE = 'P0002';"}],else:[]},
      {id:makeId(),type:'if',condition:`${quote(changed)} > 1`,then:[{id:makeId(),type:'sql',sql:"RAISE EXCEPTION 'Фильтр затронул несколько записей' USING ERRCODE = '21000';"}],else:[]},
      {id:makeId(),type:'return',expression:''},
    ];
  }else{
    if(kind==='sum' && ![20,21,23,1700].includes(column.type_oid))throw Error('Шаблон суммы поддерживает поля smallint, integer, bigint и numeric.');
    const result=unique('result_value');next.kind='function';next.returnType=kind==='sum'?([21,23].includes(column.type_oid)?'bigint':'numeric'):dataType(column);next.variables=[{name:result,type:next.returnType,initial:''}];
    const configuration=JSON.parse(JSON.stringify(settings));configuration.mode=kind==='sum'?'sum':'value';if(configuration.filter)configuration.filter.parameter=inputName;
    const key=configuration.join?findSavedLink(configuration.join,details):null;
    if(key && !joined)throw Error('Загрузите связанную таблицу.');
    const rest=(configuration.joins||[]).map((step,index)=>{
      const origin=tables[joinSides.indexOf(step.from)],target=extra[index];
      if(!origin||!target)throw Error('Загрузите таблицы соединения.');
      return {from:step.from,kind:step.link.kind,relation:target.relation,
        columns:target.details.columns,foreignKey:findSavedLink(step.link,origin.details)};
    });
    const query=buildSelect({relation,columns:details.columns,mode:configuration.mode,columnSide:configuration.column.side,columnNumber:column.number,filterSide:configuration.filter?.side||'source',filterNumber:filter?.number??null,parameter:inputName,parameters:next.parameters,variables:next.variables,routineName:next.name,join:key?{kind:configuration.join.kind,relation:joined.relation,columns:joined.details.columns,foreignKey:key}:null,extra:rest});
    next.blocks=[{id:makeId(),type:'select',target:result,query,assistant:configuration},{id:makeId(),type:'return',expression:result}];
  }
  const generated=generate(next);if(generated.errors.length)throw Error(generated.errors.join('\n'));
  return {project:next,sql:generated.sql,description:kind==='status'?'Процедура изменяет ровно одну запись; отсутствие записи или несколько изменений дают ошибку. В режиме отладки изменения откатываются.':kind==='sum'?'Функция возвращает сумму: bigint для integer/smallint, numeric для bigint/numeric. При отсутствии строк - NULL; пределы проверяет PostgreSQL.':'Функция получает одно значение по фильтру. Нет строк - NULL; несколько строк - ошибка.'};
}
