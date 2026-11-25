// Запросы каталога и конструктора используют один слот PostgreSQL на сервере.
const token = document.querySelector('meta[name="pg-debugger-token"]')?.content;
let tail = Promise.resolve(), count = 0;
function notify() { document.dispatchEvent(new CustomEvent('databaserequest', {detail:{busy:count>0}})); }

export function requestDatabase(path, body) {
  count++; notify();
  const result = tail.then(async () => {
    const response = await fetch(path, {method:'POST',headers:{'content-type':'application/json','x-pg-debugger-token':token},body:JSON.stringify(body)});
    let data;
    try { data = await response.json(); } catch { throw Error(`Неожиданный ответ сервера (${response.status}).`); }
    if (!response.ok) {document.dispatchEvent(new CustomEvent('databaseerror',{detail:{path,status:response.status,message:data.error}}));const error=Error(data.error || `Запрос не выполнен (${response.status}).`);if(Number.isInteger(data.location?.line)&&data.location.line>0&&Number.isInteger(data.location?.column)&&data.location.column>0)error.location=data.location;throw error;}
    return data;
  });
  // Ошибка предыдущего запроса не должна останавливать очередь.
  tail = result.catch(() => {});
  return result.finally(() => { count--; notify(); });
}
