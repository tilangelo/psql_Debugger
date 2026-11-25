import {emptyModel,tableHints,columnHints,updateHints,formatHintLines,parseHintLines} from './model-hints.js';

export function mountHintsEditor(root,project,relation,details,onSave){
  const panel=document.createElement('details');panel.className='hints-editor';
  const summary=document.createElement('summary');summary.textContent='Настройки модели (в проекте)';panel.append(summary);root.append(panel);
  const body=document.createElement('div');panel.append(body);
  const binding=document.createElement('p');binding.textContent=`Привязка: ${relation.schema}.${relation.name}. Подсказки не изменяют таблицу и ограничения базы.`;body.append(binding);
  const initialTable=tableHints(project.model,relation);
  function field(caption,value='',multiline=false){
    const label=document.createElement('label');label.textContent=caption;
    const input=document.createElement(multiline?'textarea':'input');input.value=value;label.append(input);body.append(label);return input;
  }
  const modelName=field('Название модели',project.model?.name||''),tableLabel=field('Понятное название таблицы',initialTable?.label||'');
  const tableDescription=field('Описание таблицы',initialTable?.description||'',true);
  const columnLabel=document.createElement('label');columnLabel.textContent='Поле для настройки';
  const column=document.createElement('select');columnLabel.append(column);body.append(columnLabel);
  for(const item of details.columns){const option=document.createElement('option');option.value=String(item.number);option.textContent=item.name;column.append(option);}
  const label=field('Понятное название поля'),description=field('Описание поля','',true);
  const values=field('Подсказки значений: значение | подпись, каждая с новой строки','',true);
  const status=document.createElement('p');status.className='hint';status.setAttribute('role','status');
  const save=document.createElement('button');save.type='button';save.textContent='Сохранить подсказки в проект';body.append(save,status);
  const selected=()=>details.columns.find(c=>String(c.number)===column.value);
  function load(){const hints=selected()?columnHints(project.model,relation,selected()):null;label.value=hints?.label||'';description.value=hints?.description||'';values.value=formatHintLines(hints?.values||[]);save.disabled=!selected();}
  column.onchange=load;load();
  save.onclick=()=>{
    try{
      const suggestions=parseHintLines(values.value);
      const model=JSON.parse(JSON.stringify(project.model||emptyModel()));model.name=modelName.value;
      const next=updateHints(model,relation,selected(),{label:tableLabel.value,description:tableDescription.value},{label:label.value,description:description.value,values:suggestions});
      onSave(next);
    }catch(error){status.textContent=error.message;}
  };
}
