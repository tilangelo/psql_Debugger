const adapters=new Map();
function current(){if(document.body.classList.contains('onboarding'))return null;return document.getElementById('view-editor').getAttribute('aria-selected')==='true'?'body':document.getElementById('view-constructor').getAttribute('aria-selected')==='true'?'project':null;}
export function refreshHistory(){
  const scope=current(),adapter=adapters.get(scope),available=adapter?.available()||{};
  for(const action of ['undo','redo']){const button=document.getElementById(action);button.disabled=!available[action];button.title=(action==='undo'?'Отменить':'Повторить')+(scope==='body'?' правку тела; определение в базе не меняется':' действие в конструкторе');}
}
export function registerHistory(scope,adapter){adapters.set(scope,adapter);refreshHistory();}
for(const action of ['undo','redo'])document.getElementById(action).onclick=()=>{const adapter=adapters.get(current());if(adapter?.available()[action])adapter[action]();refreshHistory();};
document.addEventListener('workspaceview',refreshHistory);
new MutationObserver(refreshHistory).observe(document.body,{attributes:true,attributeFilter:['class']});
document.addEventListener('keydown',event=>{
  if(event.defaultPrevented||!(event.ctrlKey||event.metaKey)||event.altKey||event.target.closest?.('[contenteditable="true"]'))return;
  if(event.target.closest?.('input,textarea')&&!(current()==='project'&&event.target.closest('#constructor-view')))return;
  const key=event.code==='KeyZ'?'z':event.code==='KeyY'?'y':event.key.toLowerCase(),action=key==='z'?(event.shiftKey?'redo':'undo'):key==='y'?'redo':null;
  if(action){event.preventDefault();document.getElementById(action).click();}
});
