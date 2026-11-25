import {declarationIssues,fillTypeSelect,readTypeChoice,typeSql} from './data-types.js';

// Пустые и временно неверные поля сохраняем как часть проекта; SQL до исправления заблокирован.
export function mountDeclarations({roots,getProject,getTypes=()=>[],onChange,onBlur=()=>{}}){
  let legacy=null;const collapsed={parameters:true,variables:true};
  function render(){
    const project=getProject();
    for(const key of ['parameters','variables']){
      const root=roots[key];root.replaceChildren();
      const heading=document.createElement('div');heading.className='declaration-heading';
      const title=document.createElement('h2');title.textContent=key==='parameters'?'Параметры':'Переменные';
      const add=document.createElement('button');add.type='button';add.textContent='Добавить';add.disabled=project[key].length>=100;
      add.onclick=()=>{collapsed[key]=false;const items=[...getProject()[key],{name:'',type:'integer',...(key==='variables'?{initial:''}:{})}];change(key,items);root.querySelector(`#declaration-${key}-${items.length-1}-name`)?.focus();};
      const fold=document.createElement('button');fold.type='button';fold.className='declaration-fold';fold.textContent=collapsed[key]?'Развернуть':'Свернуть';fold.setAttribute('aria-expanded',String(!collapsed[key]));fold.setAttribute('aria-controls',key+'-fields');
      fold.onclick=()=>{collapsed[key]=!collapsed[key];root.querySelector('.declaration-fields').hidden=collapsed[key];fold.textContent=collapsed[key]?'Развернуть':'Свернуть';fold.setAttribute('aria-expanded',String(!collapsed[key]));};
      const actions=document.createElement('div');actions.className='declaration-heading-actions';actions.append(add,fold);heading.append(title,actions);root.append(heading);
      const summary=document.createElement('p');summary.className='declaration-summary';root.append(summary);
      const content=document.createElement('div');content.id=key+'-fields';content.className='declaration-fields';content.hidden=collapsed[key];root.append(content);
      if(!project[key].length){const empty=document.createElement('p');empty.className='hint';empty.textContent=key==='parameters'?'Входных параметров нет.':'Локальные переменные не объявлены.';content.append(empty);}
      for(const [index,item] of project[key].entries()){
        const row=document.createElement('div');row.className='declaration-row';row.dataset.declaration=key;row.dataset.index=index;
        for(const [field,caption] of [['name','Имя'],['type','Тип'],...(key==='variables'?[['initial','Начальное SQL-выражение']]:[])]){
          const label=document.createElement('label');label.textContent=caption;
          const input=document.createElement(field==='type'?'select':field==='initial'?'textarea':'input');
          input.id=`declaration-${key}-${index}-${field}`;input.dataset.declarationField=field;input.spellcheck=false;
          if(field==='type'){
            fillTypeSelect(input,item.type,getTypes());
          }else input.value=item[field]||'';
          if(field==='initial'){input.rows=2;}
          const error=document.createElement('span');error.id=input.id+'-error';error.className='field-error';error.setAttribute('aria-live','polite');input.setAttribute('aria-describedby',error.id);
          const edit=()=>{const value=field==='type'?readTypeChoice(input.value):input.value;const items=getProject()[key].map((v,i)=>i===index?{...v,[field]:value}:v);legacy=null;onChange(key,items,{group:`declaration:${key}:${index}:${field}`});errors();};
          if(field==='type')input.onchange=()=>{edit();onBlur();};else{input.oninput=edit;input.onchange=onBlur;}
          label.append(input,error);row.append(label);
        }
        const actions=document.createElement('div');actions.className='declaration-actions';
        for(const [text,delta] of [['↑',-1],['↓',1]]){const button=document.createElement('button');button.type='button';button.textContent=text;button.setAttribute('aria-label',(delta<0?'Выше':'Ниже')+' - '+(item.name||'без имени'));button.disabled=index+delta<0||index+delta>=project[key].length;button.onclick=()=>{const items=[...getProject()[key]];[items[index],items[index+delta]]=[items[index+delta],items[index]];change(key,items);};actions.append(button);}
        const remove=document.createElement('button');remove.type='button';remove.textContent='Удалить';remove.setAttribute('aria-label','Удалить '+(item.name||'объявление без имени'));remove.onclick=()=>change(key,getProject()[key].filter((_,i)=>i!==index));actions.append(remove);row.append(actions);content.append(row);
      }
      if(legacy?.[key]){const review=document.createElement('details'),summary=document.createElement('summary'),pre=document.createElement('pre');summary.textContent='Объявления старого черновика - для сверки';pre.textContent=legacy[key];review.append(summary,pre);content.append(review);}
    }
    errors();
  }
  function errors(){
    const issues=declarationIssues(getProject());
    for(const key of ['parameters','variables']){
      const count=new Set(issues.filter(e=>e.key===key).map(e=>e.index+':'+e.field)).size;
      const summary=roots[key].querySelector('.declaration-summary');summary.textContent=`Объявлений: ${getProject()[key].length}`+(count?` · Ошибок: ${count}`:'');summary.dataset.errors=String(!!count);
    }
    for(const key of ['parameters','variables'])for(const input of roots[key].querySelectorAll('[data-declaration-field]')){
      const index=Number(input.closest('.declaration-row').dataset.index),field=input.dataset.declarationField;
      const text=[...new Set(issues.filter(e=>e.key===key&&e.index===index&&e.field===field).map(e=>e.message))].join(' ');
      input.setAttribute('aria-invalid',String(!!text));document.getElementById(input.id+'-error').textContent=text;
    }
  }
  function change(key,items){legacy=null;onChange(key,items,{});render();onBlur();}
  return {setProject(ui){legacy=ui&&ui.declarationsVersion!==2?{parameters:ui.parametersText,variables:ui.variablesText}:null;render();},refresh:render,refreshErrors:errors,
    snapshot(){const p=getProject();return {...(legacy?{}:{declarationsVersion:2}),parametersText:legacy?.parameters??p.parameters.map(v=>`${v.name} : ${typeSql(v.type)}`).join('\n'),variablesText:legacy?.variables??p.variables.map(v=>`${v.name} : ${typeSql(v.type)}${v.initial?' = '+v.initial:''}`).join('\n')};}
  };
}
