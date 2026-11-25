import {EditorState,Compartment,Transaction} from '@codemirror/state';
import {EditorView,keymap,lineNumbers,highlightActiveLine,highlightActiveLineGutter,drawSelection} from '@codemirror/view';
import {defaultKeymap,history,historyKeymap,indentWithTab,undo,redo,undoDepth,redoDepth,isolateHistory} from '@codemirror/commands';
import {search,searchKeymap,openSearchPanel,highlightSelectionMatches} from '@codemirror/search';
import {autocompletion,completionKeymap,startCompletion,closeCompletion,closeBrackets,closeBracketsKeymap,completionStatus} from '@codemirror/autocomplete';
import {bracketMatching,indentOnInput,syntaxHighlighting,defaultHighlightStyle,HighlightStyle,indentUnit} from '@codemirror/language';
import {sql,PostgreSQL,keywordCompletionSource} from '@codemirror/lang-sql';

const syntaxColors={
  '#404740':'#b2bed0','#708':'#cbb4ff','#219':'#b8ceff','#164':'#a7ddb2',
  '#a11':'#f5bf9d','#e40':'#ffca81','#00f':'#a4c7ff','#30a':'#cbb4ff',
  '#085':'#8bdcc7','#167':'#8bdcc7','#256':'#abdce8','#00c':'#a4c7ff',
  '#940':'#a6b5c8','#f00':'#ffaaa6',
};
const darkSyntax=HighlightStyle.define(defaultHighlightStyle.specs.map(spec=>
  spec.color?{...spec,color:syntaxColors[spec.color]||'#edf2f8'}:spec
));

export function createCodeEditor(parent,{onChange=()=>{},complete=()=>null,nonce='',label='Код PL/pgSQL',height='380px',historyCommands=null}={}){
  const writable=new Compartment(),wrapping=new Compartment();let readonly=true,silent=false,wrapped=false,refreshPending=false,disposed=false;
  const keywords=keywordCompletionSource(PostgreSQL,true);
  const extensions=[
    lineNumbers(),highlightActiveLineGutter(),highlightActiveLine(),drawSelection(),history(),
    sql({dialect:PostgreSQL}),syntaxHighlighting(darkSyntax,{fallback:true}),
    bracketMatching(),closeBrackets(),indentOnInput(),indentUnit.of('    '),
    search({top:true}),highlightSelectionMatches(),
    autocompletion({override:[context=>{
      if(readonly)return null;
      const result=complete(context.state.doc.toString(),context.pos);if(!result)return null;
      if(!context.explicit&&!result.word&&!result.qualifier.length)return null;
      const standard=result.qualifier.length?[]:(keywords(context)?.options||[]);
      return {from:result.from,options:[...result.options,...standard],filter:true};
    }]}),
    keymap.of([...(historyCommands?[{key:'Mod-z',run:()=>{historyCommands.undo();return true;}},{key:'Mod-Shift-z',run:()=>{historyCommands.redo();return true;}},{key:'Mod-y',run:()=>{historyCommands.redo();return true;}}]:[]),...closeBracketsKeymap,...completionKeymap,...searchKeymap,...defaultKeymap,...historyKeymap,indentWithTab]),
    writable.of([EditorState.readOnly.of(true),EditorView.editable.of(false)]),wrapping.of([]),
    EditorView.cspNonce.of(nonce),EditorView.contentAttributes.of({'aria-label':label,'spellcheck':'false'}),
    EditorState.phrases.of({'Find':'Найти','Replace':'Заменить','next':'Далее','previous':'Назад','all':'Все','match case':'Учитывать регистр','by word':'Слова целиком','regexp':'Регулярное выражение','replace':'Заменить','replace all':'Заменить все','close':'Закрыть','Go to line':'Перейти к строке','go':'Перейти','No completions':'Нет подсказок','Completions':'Подсказки'}),
    EditorView.theme({
      '&':{height,fontSize:'15px',color:'#edf2f8',backgroundColor:'#10151e'},
      '.cm-scroller':{overflow:'auto',fontFamily:'Consolas, "Liberation Mono", monospace',lineHeight:'1.65'},
      '.cm-content':{padding:'10px 0'},
      '.cm-gutters':{backgroundColor:'#181f2b',color:'#b2bed0',borderRight:'1px solid #354459'},
      '.cm-cursor':{borderLeftColor:'#edf2f8'},
      '&.cm-focused':{outline:'2px solid #8bdcc7',outlineOffset:'-1px'},
      '.cm-tooltip':{maxWidth:'min(600px, 90vw)'},
      '.cm-completionInfo':{whiteSpace:'pre-wrap',maxWidth:'420px'},
    },{dark:true}),
    EditorView.updateListener.of(update=>{if(update.docChanged&&!silent)onChange();}),
  ];
  const view=new EditorView({parent,state:EditorState.create({doc:'',extensions})});
  return {
    getValue:()=>view.state.doc.toString(),
    getCursor(){const pos=view.state.selection.main.head,line=view.state.doc.lineAt(pos);return {line:line.number,column:[...line.text.slice(0,pos-line.from)].length+1};},
    canUndo:()=>!readonly&&undoDepth(view.state)>0,
    canRedo:()=>!readonly&&redoDepth(view.state)>0,
    undo:()=>!readonly&&undo(view),redo:()=>!readonly&&redo(view),
    replaceValue(value){if(value===view.state.doc.toString())return;view.dispatch({changes:{from:0,to:view.state.doc.length,insert:value},annotations:[Transaction.userEvent.of('input'),isolateHistory.of('full')]});},
    setValue(value){const wasReadOnly=readonly,wasWrapped=wrapped;silent=true;try{view.setState(EditorState.create({doc:value,extensions}));}finally{silent=false;}readonly=true;wrapped=false;this.setReadOnly(wasReadOnly);this.setWrapping(wasWrapped);},
    setReadOnly(value){const next=!!value;if(next===readonly)return;readonly=next;if(readonly)closeCompletion(view);view.dispatch({effects:writable.reconfigure([EditorState.readOnly.of(readonly),EditorView.editable.of(!readonly)])});},
    focus:()=>view.focus(),search:()=>openSearchPanel(view),
    complete(){view.focus();startCompletion(view);},
    invalidateCompletions(){if(refreshPending||disposed)return;refreshPending=true;queueMicrotask(()=>{refreshPending=false;if(disposed)return;const open=completionStatus(view.state)!==null;closeCompletion(view);if(open&&view.hasFocus&&!readonly)startCompletion(view);});},
    setWrapping(value){const next=!!value;if(next===wrapped)return;wrapped=next;view.dispatch({effects:wrapping.reconfigure(wrapped?EditorView.lineWrapping:[])});},
    goTo(line,column=1){if(!Number.isInteger(line)||line<1||line>view.state.doc.lines)return false;const row=view.state.doc.line(line);const offset=[...row.text].slice(0,Math.max(0,column-1)).join('').length;const anchor=Math.min(row.to,row.from+offset);view.dispatch({selection:{anchor},effects:EditorView.scrollIntoView(anchor,{y:'center'})});view.focus();return true;},
    destroy(){disposed=true;view.destroy();},
  };
}
