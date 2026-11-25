import {createTemplate} from './templates.js';
export function mountTemplatePanel(root,getInput,onApply){
  const panel=document.createElement('details');panel.className='template-panel';root.append(panel);
  const summary=document.createElement('summary');summary.textContent='Предметные шаблоны';panel.append(summary);
  const body=document.createElement('div');panel.append(body);
  const label=document.createElement('label');label.textContent='Шаблон';const select=document.createElement('select');label.append(select);body.append(label);
  for(const [value,text]of [['lookup','Получить значение по идентификатору'],['sum','Посчитать сумму поля'],['status','Изменить статус или поле']]){const option=document.createElement('option');option.value=value;option.textContent=text;select.append(option);}
  const preview=document.createElement('button');preview.type='button';preview.textContent='Предпросмотр шаблона';body.append(preview);
  const status=document.createElement('p');status.className='hint';status.setAttribute('role','status');body.append(status);
  const code=document.createElement('pre');code.hidden=true;code.tabIndex=0;code.setAttribute('aria-label','Код шаблона');body.append(code);
  const apply=document.createElement('button');apply.type='button';apply.textContent='Заменить проект шаблоном';apply.hidden=true;body.append(apply);
  let pending=null,base=null,disabled=true;
  function invalidate(){pending=null;code.hidden=true;apply.hidden=true;}
  select.onchange=invalidate;
  preview.onclick=()=>{
    try{const input=getInput();pending=createTemplate(input.project,input.settings,input.context,select.value);base=JSON.stringify(input.project);code.textContent=pending.sql;code.hidden=false;apply.hidden=false;status.textContent=pending.description+' Будут заменены блоки, параметры и переменные черновика. Сохранение определения в базе выполняется отдельно.';}
    catch(error){invalidate();status.textContent=error.message;}
  };
  apply.onclick=()=>{
    if(!pending||disabled)return;
    if(base!==JSON.stringify(getInput().project)){invalidate();status.textContent='Проект изменился. Повторите предпросмотр.';return;}
    onApply(pending.project);
  };
  return {invalidate,setDisabled(value){disabled=value;select.disabled=value;preview.disabled=value;apply.disabled=value;}};
}
