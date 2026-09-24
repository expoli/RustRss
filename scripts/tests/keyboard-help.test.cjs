const {test}=require('node:test');
const assert=require('node:assert/strict');
const vm=require('node:vm');
const fs=require('node:fs');
const source=fs.readFileSync('ui/app.js','utf8');
const start=source.indexOf('function onGlobalKeydown(');
const handler=source.slice(start,source.indexOf('\n}',start)+2);
function fixture(code=handler){
 const nodes=new Map();let opened=0,moved=0;
 const el=id=>{
  if(id==='ctx-menu')return null;
  if(!nodes.has(id))nodes.set(id,{open:false,classList:{contains:()=>true},showModal(){this.open=true;opened++;}});
  return nodes.get(id);
 };
 const document={activeElement:{tagName:'BODY'}};
 const context=vm.createContext({el,document,tagPickerOpen:()=>false,move:()=>moved++});
 vm.runInContext(code,context);
 return {el,document,key:key=>context.onGlobalKeydown({key,preventDefault(){}}),opened:()=>opened,moved:()=>moved};
}
test('question mark opens keyboard help, with a missing-binding negative control',()=>{
 const run=code=>{const f=fixture(code);f.key('?');assert.equal(f.opened(),1);};
 run(handler);
 assert.throws(()=>run(handler.replace(/case '\?':[^\n]+/,'')),assert.AssertionError);
});
test('help never opens while typing or over settings and never navigates behind its dialog',()=>{
 const f=fixture();f.document.activeElement.tagName='INPUT';f.key('?');assert.equal(f.opened(),0);
 f.document.activeElement.tagName='BODY';f.el('settings-overlay').classList.contains=()=>false;f.key('?');assert.equal(f.opened(),0);
 f.el('settings-overlay').classList.contains=()=>true;f.el('keyboard-help').open=true;f.key('j');assert.equal(f.moved(),0);
});
test('sidebar rows activate by Enter or Space without activating nested controls twice',()=>{
 const i=source.indexOf('function bindSidebarKeyboard(');
 const code=source.slice(i,source.indexOf('\n}',i)+2);
 let listener,clicks=0,stops=0;
 const row={setAttribute(k,v){this[k]=v;},addEventListener(k,fn){listener=fn;},click(){clicks++;}};
 const c=vm.createContext({row});vm.runInContext(code+';bindSidebarKeyboard(row)',c);
 assert.equal(row.tabIndex,0);assert.equal(row.role,'button');
 for(const key of ['Enter',' '])listener({key,target:row,preventDefault(){},stopPropagation(){stops++;}});
 listener({key:'Enter',target:{},preventDefault(){throw Error('nested control intercepted');}});
 assert.equal(clicks,2);assert.equal(stops,2);
});
test('article navigation moves focus away from sidebar activation targets',()=>{
 for(const name of ['move','jump']){
  const i=source.indexOf(`function ${name}(`);const code=source.slice(i,source.indexOf('\n}',i)+2);
  let focused=0,opened=0;
  const c=vm.createContext({state:{entries:[{id:1}],selectedId:1,settings:{}},el:id=>{assert.equal(id,'entries');return {focus(){focused++;}};},openEntry:()=>opened++});
  vm.runInContext(code+`;${name}(1)`,c);assert.equal(focused,1);assert.equal(opened,1);
 }
});
