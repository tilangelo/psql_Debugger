// Проверки основаны на состоянии сервера; сообщения PostgreSQL не разбираются как команды.
export function canEnableDebugger(catalog){
  const setup=catalog?.debugger_setup;
  return !!setup&&!setup.extension_created&&setup.package_available&&setup.superuser&&!setup.read_only;
}
export function setupChecks(catalog){
  const s=catalog?.debugger_setup;
  if(!s)return null;
  let extension;
  if(s.extension_created){
    extension=s.package_available
      ?['ready','Создано в выбранной базе.']
      :['warning','Расширение создано, но его файлы недоступны. Проверьте пакет pldebugger.'];
  }else if(!s.package_available){
    extension=['blocked','Сервер не видит файлы pldbgapi. Установите пакет pldebugger для указанной версии PostgreSQL на компьютере, где запущена база, затем обновите проверку.'];
  }else if(s.read_only){
    extension=['blocked','База только для чтения. Подключитесь к базе с доступом на запись.'];
  }else if(s.superuser){
    extension=['blocked','Файлы расширения доступны. Нажмите «Включить в этой базе».'];
  }else{
    extension=['blocked','Включить расширение может суперпользователь PostgreSQL.'];
  }
  const preload=s.preload===null?['warning','У этой роли нет доступа к настройке загрузки. Плагин проверится при запуске.']
    :s.preload?['ready','Плагин указан в настройках запуска.']
    :s.restart_pending?['blocked','Настройка ожидает перезапуска. Проверьте, что plugin_debugger указан в конфигурации.']
    :['blocked','Добавьте plugin_debugger в shared_preload_libraries и перезапустите PostgreSQL.'];
  const access=!s.extension_created?['warning','Проверим после включения.']:s.api_allowed
    ?['ready','Доступ есть.']
    :['blocked','Нет доступа. Нужны права USAGE на схему и EXECUTE на функции pldbgapi.'];
  return {extension,preload,access};
}
export function readiness(catalog,{includeObject=true}={}){
  if(!catalog)return [];
  const code=catalog.debugger_code||'unknown';
  const row=(label,state,message)=>({label,state,message});
  const rows=[row('Подключение','ready',`PostgreSQL ${catalog.server_version}. База: ${catalog.database}. Пользователь: ${catalog.current_user}.`)];
  const setup=setupChecks(catalog);
  if(setup){
    rows.push(row('Расширение pldbgapi',...setup.extension));
    rows.push(row('Плагин plugin_debugger',...setup.preload));
    rows.push(row('Доступ к расширению',...setup.access));
  }else{
    const known=['configured','plugin_not_configured','unverified','access_denied'].includes(code);
    rows.push(row('Расширение pldbgapi',code==='missing_extension'?'blocked':known?'ready':'warning',
      code==='missing_extension'?'Не создано в этой базе. Администратору нужно выполнить CREATE EXTENSION pldbgapi в выбранной базе.':known?'Найдено в выбранной базе.':'Не удалось проверить. Обновите проверку; при повторной ошибке откройте диагностический отчёт.'));
    rows.push(row('Плагин plugin_debugger',code==='configured'?'ready':code==='plugin_not_configured'?'blocked':'warning',
      code==='configured'?'Указан в настройке предварительной загрузки PostgreSQL.':code==='plugin_not_configured'?'Администратору нужно добавить plugin_debugger в shared_preload_libraries и перезапустить PostgreSQL.':'Загрузка проверится при запуске.'));
    rows.push(row('Доступ к расширению',code==='access_denied'?'blocked':['configured','plugin_not_configured','unverified'].includes(code)?'ready':'warning',
      code==='access_denied'?'Недостаточно прав на схему или функции pldbgapi. Обратитесь к администратору.':['configured','plugin_not_configured','unverified'].includes(code)?'Базовая функция отладки доступна роли.':'Проверка доступа не завершена.'));
  }
  if(!includeObject)return rows;
  if(catalog.selected){
    const routine=catalog.routines.find(r=>r.oid===catalog.selected.oid);
    rows.push(row('Права выбранного объекта',routine?.debug_allowed?'ready':'blocked',routine?.debug_allowed?'Отладка разрешена.':(routine?.owner?'Владелец: '+routine.owner+'. ':'')+'Войдите как владелец или суперпользователь.'));
    if(Array.isArray(catalog.selected.trigger_targets)){
      const found=catalog.selected.trigger_targets.length;
      rows.push(row('Триггер таблицы',found?'ready':'blocked',found?'Выберите привязку к таблице и операцию.':'Нет доступной немедленной построчной привязки к обычной таблице. Для UPDATE и DELETE нужен первичный ключ.'));
      return rows;
    }
    const unsupported=catalog.selected.arguments.filter(a=>a.mode==='VARIADIC'||(['IN','INOUT'].includes(a.mode)&&!a.supported&&!a.has_default));
    const onlyDefault=catalog.selected.arguments.some(a=>['IN','INOUT'].includes(a.mode)&&!a.supported&&a.has_default);
    rows.push(row('Входные аргументы',unsupported.length?'blocked':onlyDefault?'warning':'ready',
      unsupported.length?'Не поддерживаются: '+unsupported.map(a=>`${a.name||'без имени'} (${a.type})`).join(', ')+'. Проверьте вызов через SQL в тестовой базе.':
      onlyDefault?'Для отмеченных типов доступно только значение по умолчанию.':'Поддерживаются.'));
  }else rows.push(row('Выбор объекта','warning','Выберите функцию или процедуру, чтобы проверить права и аргументы.'));
  return rows;
}
export function blockingReason(catalog){return readiness(catalog).filter(r=>r.state==='blocked').map(r=>`${r.label}: ${r.message}`).join('\n');}
