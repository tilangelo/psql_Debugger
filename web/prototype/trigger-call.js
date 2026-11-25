export function mountTriggerCall(anchor,onChange){
  const section=document.createElement('section');
  section.className='trigger-call';section.hidden=true;
  section.innerHTML=`<div class="preset-row"><label>Триггер<select data-trigger></select></label>
    <label>Операция<select data-operation></select></label></div>
    <details open><summary>Данные операции</summary><div data-trigger-fields></div></details>
    <p role="status"></p>`;
  anchor.after(section);
  const select=section.querySelector('[data-trigger]'),operation=section.querySelector('[data-operation]');
  const fields=section.querySelector('[data-trigger-fields]'),status=section.querySelector('[role=status]');
  let targets=null,locked=false,entries=[];
  const selected=()=>targets?.find(target=>String(target.oid)===select.value);

  function options(control,placeholder,items){
    control.replaceChildren(new Option(placeholder,''));
    for(const [value,label] of items)control.add(new Option(label,value));
    if(items.length===1)control.selectedIndex=1;
  }
  function controls(){
    select.disabled=locked||!targets?.length;
    operation.disabled=locked||!selected();
    section.querySelector('details').hidden=!operation.value;
    for(const {mode,input} of entries){
      mode.disabled=locked;
      input.hidden=mode.value!=='value';
      input.disabled=locked||input.hidden;
    }
  }
  function reason(){
    if(!targets)return '';
    if(!selected())return 'Выберите триггер.';
    if(!operation.value)return 'Выберите операцию.';
    if(entries.some(entry=>entry.key&&entry.mode.value==='omit'))return 'Задайте все поля первичного ключа.';
    if(operation.value==='UPDATE'&&!entries.some(entry=>!entry.key&&entry.mode.value!=='omit'))return 'Выберите хотя бы одно изменяемое поле.';
    return '';
  }
  function updated(){controls();status.textContent=targets?.length?reason():'';onChange();}
  function renderEntries(){
    entries=[];fields.replaceChildren();
    const target=selected(),event=operation.value;
    if(!target||!event){controls();return;}
    function fieldset(title,columns,key){
      const group=document.createElement('fieldset'),legend=document.createElement('legend');
      legend.textContent=title;group.append(legend);
      for(const column of columns){
        const row=document.createElement('div'),label=document.createElement('label');
        const mode=document.createElement('select'),input=document.createElement('input');
        row.className='preset-row';label.textContent=`${column.name} (${column.type_name})`;
        mode.add(new Option(key?'Не задано':'Пропустить','omit'));
        mode.add(new Option('Значение','value'));
        if(!key)mode.add(new Option('SQL NULL','null'));
        mode.onchange=()=>{updated();if(!input.hidden)input.focus();};
        input.type='text';input.setAttribute('aria-label',column.name+' - значение');
        input.spellcheck=false;input.autocomplete='off';
        label.append(mode);row.append(label,input);group.append(row);
        entries.push({column,key,mode,input});
      }
      fields.append(group);
    }
    if(event!=='INSERT')fieldset('Первичный ключ',target.primary_key.map(name=>target.columns.find(column=>column.name===name)),true);
    if(event!=='DELETE')fieldset(event==='INSERT'?'Значения новой строки':'Изменяемые поля',target.columns.filter(column=>column.writable),false);
    controls();
  }
  select.onchange=()=>{
    options(operation,'Выберите операцию',(selected()?.operations||[]).map(value=>[value,value]));
    renderEntries();updated();
  };
  operation.onchange=()=>{renderEntries();updated();};
  return {
    setCatalog(catalog){
      targets=catalog?.selected?.trigger_targets||null;
      section.hidden=!targets?.length;
      options(select,'Выберите триггер',(targets||[]).map(target=>[String(target.oid),`${target.schema}.${target.table} · ${target.name}`]));
      options(operation,'Выберите операцию',(selected()?.operations||[]).map(value=>[value,value]));
      renderEntries();status.textContent=targets?.length?reason():'';
    },
    setLocked(value){locked=value;controls();},
    isTrigger:()=>targets!==null,
    reason,
    payload(){
      if(!targets)return null;
      const target=selected();
      const key=[],values=[];
      for(const entry of entries){
        if(entry.mode.value==='omit')continue;
        (entry.key?key:values).push([entry.column.name,entry.mode.value==='null'?null:entry.input.value]);
      }
      return {oid:target.oid,expected_definition:target.definition,operation:operation.value,
        key:Object.fromEntries(key),values:Object.fromEntries(values)};
    },
  };
}
