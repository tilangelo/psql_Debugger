// Подсказка создаёт обычный текст SELECT. В черновике по-прежнему хранится query.
import {joinSides} from './select-settings.js';
export const selectModes = [
  ['value', 'Значение поля'], ['count', 'Число строк'],
  ['sum', 'Сумма (SUM)'], ['min', 'Минимум (MIN)'], ['max', 'Максимум (MAX)'],
];
const numeric = new Set([20, 21, 23, 700, 701, 1700]);
const ordered = new Set([...numeric, 25, 1042, 1043, 1082, 1083, 1114, 1184, 1186, 1266, 790]);
const quote = name => {
  if (typeof name !== 'string' || !name || name.includes('\0')) throw Error('Некорректное имя SQL.');
  return '"' + name.replace(/"/g, '""') + '"';
};

export function columnAllowed(mode, column) {
  return mode === 'value' || (mode === 'sum' ? numeric : ordered).has(column.type_oid);
}

export function buildSelect({relation, columns, mode, columnNumber, columnSide='source', filterNumber, filterSide='source', parameter, parameters, variables=[], routineName, join=null, extra=[]}) {
  if (!relation || !selectModes.some(([value]) => value === mode)) throw Error('Выберите таблицу и операцию.');
  if(!Array.isArray(extra)||extra.length>3||(!join&&extra.length))throw Error('Допускается до четырёх соединений.');
  const steps=join?[{...join,from:'source'},...extra]:[];
  const tables=new Map([['source',{relation,columns}]]);
  for(const [index,step] of steps.entries())tables.set(joinSides[index+1],step);
  if(!tables.has(columnSide)||!tables.has(filterSide))throw Error('Выберите связь для поля связанной таблицы.');
  const column = tables.get(columnSide).columns.find(c => c.number === columnNumber);
  if (mode === 'count' && [...tables.values()].some(table=>!table.relation.can_select_all)) throw Error('COUNT(*) требует SELECT на все таблицы запроса.');
  if (mode !== 'count' && (!column || !columnAllowed(mode, column))) throw Error('Выберите поле подходящего типа.');
  const reserved = new Set([routineName,...parameters.map(p=>p.name),...variables.map(v=>v.name)]);
  function nextAlias(base){let alias=base;while(reserved.has(alias))alias+='_1';reserved.add(alias);return alias;}
  const alias=nextAlias('__pgdbg_row');
  const aliases=new Map([['source',alias]]);
  const field = (c,side='source') => `${quote(aliases.get(side))}.${quote(c.name)}`;
  let from = `FROM ${quote(relation.schema)}.${quote(relation.name)} AS ${quote(alias)}`;
  for(const [index,join] of steps.entries()){
    const side=joinSides[index+1],joinedAlias=nextAlias(index===0?'__pgdbg_join':`__pgdbg_join_${index+1}`);
    const origin=tables.get(join.from);
    if(!aliases.has(join.from)||!origin)throw Error('Соединение ссылается на ещё не выбранную таблицу.');
    aliases.set(side,joinedAlias);
    const key=join.foreignKey;
    if(!['inner','left'].includes(join.kind) || !key || !Array.isArray(key.columns) || !key.columns.length || key.columns.length!==key.referenced_columns?.length
      || key.referenced_oid!==join.relation.oid || key.referenced_schema!==join.relation.schema || key.referenced_table!==join.relation.name)throw Error('Связь изменилась. Обновите структуру и выберите внешний ключ заново.');
    const conditions=key.columns.map((name,index)=>{
      const source=origin.columns.find(c=>c.name===name), target=join.columns.find(c=>c.name===key.referenced_columns[index]);
      if(!source || !target)throw Error('Колонки связи недоступны. Обновите структуру.');
      return `${field(source,join.from)} = ${field(target,side)}`;
    });
    from+=`\n${join.kind==='left'?'LEFT':'INNER'} JOIN ${quote(join.relation.schema)}.${quote(join.relation.name)} AS ${quote(joinedAlias)}\n    ON ${conditions.join(' AND ')}`;
  }
  if (filterNumber !== null && filterNumber !== undefined) {
    const filter = tables.get(filterSide).columns.find(c => c.number === filterNumber);
    const index = parameters.findIndex(p => p.name === parameter);
    if (!filter || index < 0) throw Error('Выберите поле фильтра и входной параметр.');
    // Ссылка по номеру не путает параметр с колонкой.
    from += `\nWHERE ${field(filter,filterSide)} = $${index+1}`;
  }
  if (mode === 'value') {
    // Нет строк - NULL. Больше одной строки - ошибка.
    return `SELECT (\n    SELECT ${field(column,columnSide)}\n    ${from.replace(/\n/g, '\n    ')}\n)`;
  }
  const expression = mode === 'count' ? '*' : field(column,columnSide);
  return `SELECT pg_catalog.${mode}(${expression})\n${from}`;
}
