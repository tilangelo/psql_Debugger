// Базовый отчёт не включает код, значения аргументов или произвольные поля подключения.
export function diagnosticReport({version,catalog,session,failure,includeDetails=false}){
  const routine=session?.routine||catalog?.routines.find(r=>r.oid===catalog.selected?.oid);
  const argumentTypes=catalog?.selected?.arguments.map(a=>`${a.mode} ${a.name||''}: ${a.type}`).join(', ');
  const lines=[`PostgreSQL Debugger ${version||'версия не получена'}`,`PostgreSQL: ${session?.server_version||catalog?.server_version||'не получено'}`,`База: ${session?.database||catalog?.database||'не выбрана'}`,`Пользователь: ${session?.current_user||catalog?.current_user||'не получено'}`,`Объект: ${routine?`${routine.schema||catalog?.schema}.${routine.name}`:'не выбран'}`,`Параметры (типы): ${argumentTypes||'не получены'}`,`Готовность отладки: ${session?.debugger_status||catalog?.debugger||'не проверена'}`,`Состояние: ${session?.phase||'нет сеанса'}`,`Вызов: ${session?.outcome?.call||'нет результата'}`,`Откат: ${session?.outcome?.rollback||'нет результата'}`,`SQLSTATE: ${session?.outcome?.sqlstate||'нет'}`];
  if(failure)lines.push(`Последний неудачный запрос: ${failure.path}, HTTP ${failure.status}`);
  if(includeDetails){
    if(failure?.message)lines.push('\nОшибка запроса:\n'+failure.message);
    if(session?.outcome?.text)lines.push('\nРезультат и ошибка вызова:\n'+session.outcome.text);
    const definition=session?.routine?.definition||catalog?.selected?.definition;
    if(definition)lines.push('\nОпределение:\n'+definition);
  }
  return lines.join('\n');
}
