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
 vm.runInContext(extract('fetchFailureMessage')+extract('doAddFeed'),c);await vm.runInContext('doAddFeed()',c);
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
test('fetch messages use stable codes rather than diagnostic language',()=>{
 const c=vm.createContext({t:(key,args)=>({key,args})});vm.runInContext(extract('fetchFailureMessage'),c);
 const message=(code,error)=>{c.code=code;c.detail=error;return vm.runInContext('fetchFailureMessage(code,detail)',c);};
 assert.equal(message('retry_deferred','raw').key,'fetchError.deferred');
 assert.equal(message('timeout','超时: backend').key,'fetchError.timeout');
 assert.equal(message('timeout','timeout: backend').key,'fetchError.timeout');
 assert.equal(message('connection_error','raw').key,'fetchError.connection');
 assert.equal(message('http_429','HTTP 429').key,'fetchError.rateLimited');
 assert.equal(message('http_503','HTTP 503').args.status,'503');
 assert.equal(message('parse_error','解析失败').key,'fetchError.parse');
 assert.equal(message('http_200','legacy body error').key,'fetchError.body');
 assert.equal(message('future_code','diagnostic retained'),'diagnostic retained');
});
test('IPC preserves structured discovery code while retaining legacy string errors',async()=>{
 let failure={code:'timeout',message:'raw timeout'};
 const c=vm.createContext({window:{__TAURI__:{core:{invoke:async()=>{throw failure;}}}}});
 vm.runInContext(extract('invoke'),c);
 await assert.rejects(vm.runInContext("invoke('discover_feed')",c),e=>e.code==='timeout'&&e.message==='raw timeout');
 failure='legacy failure';await assert.rejects(vm.runInContext("invoke('other')",c),e=>e.message==='legacy failure');
});

async function saveProxy(mode, failure) {
 const fields = Object.fromEntries(['set-proxy-save','set-proxy-mode','set-proxy-url','set-proxy-bypass','set-proxy-status'].map(id=>[id,{value:''}]));
 fields['set-proxy-mode'].value=mode;
 fields['set-proxy-url'].value=' http://127.0.0.1:8080 ';
 fields['set-proxy-bypass'].value=' localhost ';
 let sent, accepted=false;
 const c=vm.createContext({el:id=>fields[id],t:key=>key,acceptSettings:()=>{accepted=true;},
   invoke:async(command,args)=>{sent={command,args};if(failure)throw Error(failure);return {};}});
 const marker="el('set-proxy-save').addEventListener('click', async () => {";
 const start=source.indexOf(marker);assert.notEqual(start,-1);
 const end=source.indexOf("\n  });",start);
 await vm.runInContext('(async()=>{'+source.slice(start+marker.length,end)+'})()',c);
 return {fields,sent,accepted};
}
test('proxy save trims custom values and clears inactive proxy fields',async()=>{
 const custom=await saveProxy('custom');
 assert.equal(custom.sent.command,'set_proxy_config');
 assert.equal(custom.sent.args.config.url,'http://127.0.0.1:8080');
 assert.equal(custom.sent.args.config.no_proxy,'localhost');
 assert.equal(custom.accepted,true);assert.equal(custom.fields['set-proxy-save'].disabled,false);
 for(const mode of ['direct','environment']){
  const r=await saveProxy(mode);assert.equal(r.sent.args.config.url,'');assert.equal(r.sent.args.config.no_proxy,'');
 }
});
test('proxy rejection leaves accepted settings alone and releases the save button',async()=>{
 const r=await saveProxy('custom','proxy_credentials_not_supported');
 assert.equal(r.accepted,false);assert.equal(r.fields['set-proxy-save'].disabled,false);
 assert.equal(r.fields['set-proxy-status'].textContent,'settings.proxy.credentials');
});
