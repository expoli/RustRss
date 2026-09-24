const {test}=require('node:test');
const assert=require('node:assert/strict');
const vm=require('node:vm');
const fs=require('node:fs');
const source=fs.readFileSync('ui/app.js','utf8');
function extract(name){const i=source.indexOf(`function ${name}(`);assert.notEqual(i,-1);return (source.slice(i-6,i)==='async '?'async ':'')+source.slice(i,source.indexOf('\n}',i)+2);}
test('feed reorder rejects self, foreign groups and stale IDs before IPC',async()=>{
 const calls=[];let refresh=0;
 const c=vm.createContext({state:{feeds:[{id:1,folder_id:null},{id:2,folder_id:null},{id:3,folder_id:9}]},
 invoke:async(cmd,args)=>calls.push({cmd,args}),refreshCounts:async()=>refresh++,setStatus:()=>{},t:k=>k});
 vm.runInContext(extract('feedOrderTarget')+'\n'+extract('moveFeed'),c);
 for(const [a,b] of [[1,1],[1,3],[1,999],[999,1]])await vm.runInContext(`moveFeed(${a},${b},true)`,c);
 assert.equal(calls.length,0);
 await vm.runInContext('moveFeed(2,1,true)',c);assert.equal(calls.length,1);assert.equal(refresh,1);
 assert.equal(calls[0].cmd,'move_feed');assert.equal(calls[0].args.feedId,2);assert.equal(calls[0].args.targetId,1);assert.equal(calls[0].args.before,true);
});
test('unsubscribe cancellation cannot delete and confirmation sends exact feed ID',async()=>{
 let confirmed=false;const calls=[];
 const c=vm.createContext({confirmDialog:async()=>confirmed,t:k=>k,invoke:async(cmd,args)=>calls.push({cmd,args}),
 state:{view:{kind:'all'}},refreshCounts:async()=>{},loadEntries:async()=>{},setStatus:()=>{},log:()=>{}});
 vm.runInContext(extract('unsubscribeFeed'),c);
 await vm.runInContext("unsubscribeFeed({id:7,title:'fixture'})",c);assert.equal(calls.length,0);
 confirmed=true;await vm.runInContext("unsubscribeFeed({id:7,title:'fixture'})",c);
 assert.equal(calls.length,1);assert.equal(calls[0].cmd,'remove_feed');assert.equal(calls[0].args.feedId,7);
});
