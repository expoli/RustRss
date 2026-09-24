const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const vm=require('node:vm');
const source=fs.readFileSync('ui/app.js','utf8');
function extract(name){
 const start=source.indexOf(`function ${name}(`);assert.notEqual(start,-1,`missing ${name}`);
 return (source.slice(start-6,start)==='async '?'async ':'')+source.slice(start,source.indexOf('\n}',start)+2);
}
test('empty state distinguishes no subscriptions, search, unread and tag views',()=>{
 const state={feeds:[],view:{kind:'unread'}};const c=vm.createContext({state});
 vm.runInContext(extract('listEmptyKey'),c);
 const key=()=>vm.runInContext('listEmptyKey()',c);
 assert.equal(key(),'list.emptySubscriptions');
 state.view.kind='search';assert.equal(key(),'list.emptySearch');
 state.feeds=[{id:1}];assert.equal(key(),'list.emptySearch');
 state.view.kind='unread';assert.equal(key(),'list.emptyUnread');
 state.view.kind='tag';assert.equal(key(),'tags.empty');
 state.view.kind='feed';assert.equal(key(),'list.empty');
});
async function addResult(failures){
 const input={value:'http://fixture/feed'},button={disabled:false},statuses=[];let reloads=0;
 const c=vm.createContext({el:id=>id==='add-url'?input:button,
  invoke:async command=>command==='discover_feed'?{feed_url:input.value,via:'direct',alternatives:[]}:
   command==='add_feed'?1:{inserted:failures.length?0:1,failures},
  t:(key,args)=>({key,args}),setStatus:(text,error=false)=>statuses.push({text,error}),
  log(){},loadAll:async()=>{reloads++;}});
 vm.runInContext(extract('doAddFeed'),c);await vm.runInContext('doAddFeed()',c);
 return {input,button,statuses,reloads};
}
test('a subscribed feed whose initial fetch fails is not reported as success',async()=>{
 const r=await addResult([{error:'HTTP 503',feed_id:1}]);const last=r.statuses.at(-1);
 assert.equal(last.text.key,'status.addedFetchFailed');assert.equal(last.error,true);
 assert.equal(last.text.args.error,'HTTP 503');assert.equal(r.reloads,1);
 assert.equal(r.button.disabled,false);assert.equal(r.input.value,'');
});
test('successful initial fetch retains successful status and normal cleanup',async()=>{
 const r=await addResult([]);assert.equal(r.statuses.at(-1).text.key,'status.added');
 assert.equal(r.statuses.at(-1).error,false);assert.equal(r.reloads,1);assert.equal(r.button.disabled,false);
});
