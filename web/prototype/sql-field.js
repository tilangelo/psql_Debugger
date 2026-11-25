import {createCodeEditor} from './vendor/code-editor.js';
import {mountSqlHints} from './sql-hints.js';

export function mountSqlField(root,project,value,caption,onChange,historyCommands){
  const section=document.createElement('section');section.className='inline-sql-field';root.append(section);
  const label=document.createElement('p');label.textContent=caption;section.append(label);
  const toolbar=document.createElement('div');toolbar.className='code-toolbar';
  const find=document.createElement('button'),complete=document.createElement('button');find.textContent='Найти';complete.textContent='Подсказки · Ctrl+Space';toolbar.append(find,complete);section.append(toolbar);
  const code=document.createElement('div');code.className='sql-field-code';section.append(code);
  let hints;
  const editor=createCodeEditor(code,{height:'190px',label:caption,historyCommands,nonce:document.querySelector('meta[name="pg-debugger-token"]')?.content||'',complete:(source,pos)=>hints?.complete(source,pos),onChange:()=>onChange(editor.getValue())});
  hints=mountSqlHints(section,()=>project.parameters,()=>editor.invalidateCompletions(),{variables:()=>project.variables,initialModel:project.model,autoLoad:false});
  editor.setValue(value);editor.setReadOnly(false);find.onclick=()=>editor.search();complete.onclick=()=>editor.complete();
  return {setValue:value=>editor.setValue(value),getCursor:()=>editor.getCursor(),focusAt:cursor=>editor.goTo(cursor.line,cursor.column),destroy(){hints.destroy();editor.destroy();section.remove();}};
}
