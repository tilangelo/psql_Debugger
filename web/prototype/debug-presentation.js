// Сравниваем только разные остановки одного сеанса и кадра. Опрос и точки не являются шагом.
export function createPauseComparison(){
  let identity=null,pause=null,previous=null,rows=[];
  return {update(snapshot){
    const frame=snapshot.frame??snapshot.stack?.[0];
    const chain=(snapshot.stack||[]).map(item=>item.oid);
    const nextIdentity=JSON.stringify([snapshot.session_id,frame?.level,frame?.oid,frame?.name,chain]);
    if(identity!==nextIdentity){identity=nextIdentity;pause=null;previous=null;rows=[];}
    if(!snapshot.pause_id||snapshot.pause_id===pause)return rows;
    if(snapshot.phase!=='paused'&&snapshot.phase!=='finished')return rows;
    const next=new Map();
    rows=(snapshot.variables||[]).map(variable=>{
      const key=JSON.stringify([variable.name,variable.declaration_line]),old=previous?.get(key);
      let state='same';
      if(previous&&!old)state='added';
      else if(old&&(old.value!==variable.value||!!old.truncated!==!!variable.truncated))state='changed';
      else if(variable.truncated)state='partial';
      next.set(key,variable);
      return {...variable,state,previous:state==='changed'?old:undefined};
    });
    pause=snapshot.pause_id;previous=next;return rows;
  }};
}

// Поздний ответ команды или опроса не возвращает интерфейс к старому состоянию.
export function newerSnapshot(current,next){
  if(!next||next.unchanged||!next.session_id)return false;
  if(!current)return true;
  return next.session_id>current.session_id||(next.session_id===current.session_id&&next.revision>current.revision);
}

export function outcomeSummary(outcome){
  const calls={succeeded:['Вызов выполнен','success'],failed:['Ошибка SQL','error'],setup_failed:['Ошибка подготовки','error'],not_started:['Тело не выполнялось','neutral'],definition_changed:['Определение изменилось; вызов не выполнен','warning'],unknown:['Исход SQL не подтверждён','warning']};
  const rollbacks={confirmed:['Откат подтверждён','success'],unconfirmed:['Откат не подтверждён','warning'],not_needed:['Откат не требовался','neutral']};
  const [call,callTone]=calls[outcome?.call]||['Исход SQL неизвестен','warning'];
  const [rollback,rollbackTone]=rollbacks[outcome?.rollback]||['Статус отката неизвестен','warning'];
  return {call,callTone,rollback,rollbackTone,cancellation:outcome?.cancellation_requested?'Отмена была запрошена':null,sqlstate:outcome?.sqlstate||null};
}

export function phaseDescription(session){
  if(!session)return 'Задайте аргументы и начните отладку. Без выбранных точек вызов остановится на входе.';
  return ({starting:'Подготавливается вызов и подключение отладчика. Дождитесь остановки или отмените вызов.',running:'Выполнение продолжается. Шаг и изменение точек доступны на паузе; отмена доступна сейчас.',paused:`Следующим будет выполнен оператор на строке ${session.position?.line??'с неизвестным номером'}. «Шаг без входа» выполняет оператор целиком. «Шаг внутрь» может остановиться во вложенном PL/pgSQL-вызове.`,stopping:'Отмена запрошена. Ожидаем завершения вызова и подтверждения отката; новый запуск пока недоступен.',finished:''})[session.phase]??'Состояние отладки не определено.';
}
