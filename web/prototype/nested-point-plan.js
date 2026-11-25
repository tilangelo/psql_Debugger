export function mountNestedPointPlan(anchor,{browse,resolve,onBusy}){
  const panel=document.createElement('dialog');
  panel.className='nested-point-plan debug-dialog';
  panel.setAttribute('aria-labelledby','nested-plan-title');
  panel.innerHTML=`<div class="debug-dialog-heading"><h2 id="nested-plan-title">Вложенные точки</h2><button data-close type="button">Закрыть</button></div>
    <div class="preset-row"><label>Схема<select data-schema></select></label>
    <label>Функция или процедура<select data-routine></select></label></div>
    <form class="preset-row" hidden><label>Номер строки<input type="number" min="1" step="1" required></label>
    <button type="submit">Добавить точку</button></form>
    <pre tabindex="0" aria-label="Код вложенного объекта с номерами строк" hidden></pre>
    <ul aria-label="Подготовленные точки" hidden></ul><p role="status"></p>`;
  anchor.after(panel);
  const find=selector=>panel.querySelector(selector);
  const schema=find('[data-schema]'),routine=find('[data-routine]'),form=find('form'),line=find('input');
  const source=find('pre'),list=find('ul'),status=find('[role=status]');
  let root=null,object=null,loading=false,locked=true,running=false,generation=0;
  const groups=new Map();
  find('[data-close]').onclick=()=>panel.close();

  function controls(){
    if((!root||running)&&panel.open)panel.close();
    schema.disabled=locked||loading;
    routine.disabled=locked||loading||routine.options.length<2;
    line.disabled=form.querySelector('button').disabled=locked||loading||!object;
    list.querySelectorAll('button').forEach(button=>{button.disabled=locked||loading;});
  }
  function renderGroups(){
    list.replaceChildren();
    for(const group of groups.values()){
      const item=document.createElement('li');
      item.append(document.createTextNode(group.name+' '));
      for(const number of [...group.lines].sort((a,b)=>a-b)){
        const button=document.createElement('button');
        button.type='button';
        button.textContent=`Строка ${number} ×`;
        button.setAttribute('aria-label',`Снять точку ${group.name}, строка ${number}`);
        button.onclick=()=>{
          group.lines.delete(number);
          if(!group.lines.size)groups.delete(group.oid);
          status.textContent='';renderGroups();controls();onBusy();
        };
        item.append(button);
      }
      list.append(item);
    }
    list.hidden=!groups.size;
  }
  function clearSource(){
    object=null;line.value='';form.hidden=source.hidden=true;source.textContent='';
  }
  function showRoutines(result){
    routine.replaceChildren(new Option('Выберите объект',''));
    for(const item of result.routines){
      if(item.oid===root.selected.oid||!item.debug_allowed)continue;
      routine.add(new Option(`${item.name}(${item.signature})`,String(item.oid)));
    }
    if(routine.options.length===1)status.textContent='В схеме нет доступных вложенных объектов.';
  }
  async function request(action){
    if(loading||locked)return;
    const ticket=generation,alive=()=>ticket===generation;
    loading=true;status.textContent='';controls();onBusy();
    try{await action(alive);}
    catch(error){if(alive())status.textContent=error.message;}
    finally{
      if(alive()){loading=false;controls();onBusy();}
    }
  }
  function read(oid=null){
    return browse({connection_id:root.connection_id,database:root.database,schema:schema.value,routine_oid:oid});
  }
  schema.onchange=()=>{
    clearSource();routine.replaceChildren(new Option('Выберите объект',''));
    request(async alive=>{
      const result=await read();
      if(!alive())return;
      if(result.schema!==schema.value)throw Error('Схема изменилась. Обновите каталог.');
      showRoutines(result);
    });
  };
  routine.onchange=()=>{
    clearSource();status.textContent='';
    if(!routine.value){controls();return;}
    request(async alive=>{
      const oid=Number(routine.value),result=await read(oid);
      if(!alive())return;
      if(result.schema!==schema.value||result.selected?.oid!==oid)throw Error('Объект изменился. Выберите его заново.');
      const entry=result.routines.find(item=>item.oid===oid&&item.debug_allowed);
      if(!entry)throw Error('У пользователя больше нет прав для отладки объекта.');
      object={oid,schema:result.schema,name:`${result.schema}.${entry.name}(${entry.signature})`,
        expected_definition:result.selected.definition,source:result.selected.source};
      const rows=object.source.split('\n');
      const width=String(rows.length).length;
      source.textContent=rows.map((text,index)=>`${String(index+1).padStart(width)}  ${text}`).join('\n');
      source.scrollTop=0;line.max=rows.length;form.hidden=source.hidden=false;
    });
  };
  form.onsubmit=event=>{
    event.preventDefault();
    if(!object||!form.reportValidity())return;
    const body=object,number=line.valueAsNumber;
    request(async alive=>{
      const saved=groups.get(body.oid);
      if(saved&&saved.expected_definition!==body.expected_definition)throw Error('Определение изменилось. Снимите старые точки этого объекта и добавьте их заново.');
      const result=await resolve({source:body.source,line:number});
      if(!alive())return;
      const actual=result.line;
      const total=[...groups.values()].reduce((size,group)=>size+group.lines.size,0);
      if((!saved&&groups.size>=8)||(!saved?.lines.has(actual)&&total>=32)){
        throw Error('Можно подготовить до 32 точек в восьми вложенных объектах.');
      }
      const group=saved||{schema:body.schema,oid:body.oid,name:body.name,expected_definition:body.expected_definition,lines:new Set()};
      group.lines.add(actual);groups.set(group.oid,group);renderGroups();
      status.textContent=actual===number?`Строка ${actual} добавлена.`:`Строка ${number} относится к оператору на строке ${actual}.`;
      line.value='';
    });
  };
  return {
    setCatalog(next){
      generation++;root=next?.selected?next:null;loading=false;groups.clear();clearSource();
      panel.close();status.textContent='';schema.replaceChildren();routine.replaceChildren();
      if(root){
        for(const name of root.schemas)schema.add(new Option(name,name,false,name===root.schema));
        showRoutines(root);
      }
      renderGroups();controls();
    },
    setLocked(value,active){locked=value;running=active;controls();},
    busy:()=>loading,
    open(){if(root&&!running&&!panel.open)panel.showModal();},
    hasPoints:()=>groups.size>0,
    payload:()=>[...groups.values()].map(({schema,oid,expected_definition,lines})=>({schema,oid,expected_definition,lines:[...lines]})),
  };
}
