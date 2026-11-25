import {diagnosticReport} from './diagnostic-report.js';
const token=document.querySelector('meta[name="pg-debugger-token"]')?.content;
if(token){
  let version=null,catalog=null,session=null,failure=null;
  const panel=document.createElement('details');panel.className='diagnostics';
  panel.innerHTML='<summary>Диагностический отчёт</summary><label><input type="checkbox" id="diagnostic-details"> Включить код и подробности ошибки</label><button id="diagnostic-copy" type="button">Скопировать отчёт</button><p id="diagnostic-status" role="status">Перед отправкой просмотрите отчёт. По умолчанию код и значения аргументов не включены.</p><textarea id="diagnostic-report" readonly aria-label="Диагностический отчёт"></textarea>';
  document.getElementById('debug-readiness').append(panel);
  const text=panel.querySelector('textarea'),details=panel.querySelector('input');
  function render(){text.value=diagnosticReport({version,catalog,session,failure,includeDetails:details.checked});}
  details.onchange=render;panel.ontoggle=render;
  document.addEventListener('catalogchange',event=>{catalog=event.detail;session=null;render();});
  document.addEventListener('debugsnapshot',event=>{session=event.detail;render();});
  document.addEventListener('databaseerror',event=>{failure=event.detail;render();});
  panel.querySelector('button').onclick=async()=>{render();try{await navigator.clipboard.writeText(text.value);panel.querySelector('#diagnostic-status').textContent='Отчёт скопирован.';}catch{panel.querySelector('#diagnostic-status').textContent='Выделите и скопируйте текст отчёта вручную.';}};
  fetch('/api/info',{method:'POST',headers:{'x-pg-debugger-token':token}}).then(r=>r.json()).then(info=>{version=info.version;render();}).catch(()=>{});render();
}
