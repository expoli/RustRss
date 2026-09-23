const {test}=require('node:test');const assert=require('node:assert/strict');const fs=require('node:fs');const vm=require('node:vm');
const ctx=vm.createContext({});vm.runInContext(fs.readFileSync('ui/theme-sync.js','utf8'),ctx);
function deferred(){let resolve;return {promise:new Promise(r=>resolve=r),resolve:v=>resolve(v)};}
test('hidden polling does not read; event forces a check; unchanged revision does not apply',async()=>{
 let reads=0,applied=0;const s=ctx.RustRssThemeSync.createSync({read:async()=>{reads++;return null;},revision:()=>4,active:()=>false,apply:()=>applied++});
 await s.check();assert.equal(reads,0);await s.check(true);assert.equal(reads,1);assert.equal(applied,0);
});
test('one in-flight check coalesces bursts and catches a notification arriving during a read',async()=>{
 const d=deferred();let reads=0,revision=0;const s=ctx.RustRssThemeSync.createSync({read:()=>++reads===1?d.promise:Promise.resolve({revision:2}),revision:()=>revision,active:()=>true,apply:x=>revision=x.revision});
 const first=s.check();s.check(true);s.check(true);assert.equal(reads,1);d.resolve({revision:1});await first;
 assert.equal(reads,2);assert.equal(revision,2);
});
test('disposing drops late replies and failures do not poison the next read',async()=>{
 const d=deferred();let applied=0;const s=ctx.RustRssThemeSync.createSync({read:()=>d.promise,revision:()=>0,active:()=>true,apply:()=>applied++});
 const p=s.check();s.dispose();d.resolve({revision:1});await p;assert.equal(applied,0);
 let n=0,errors=0;const retry=ctx.RustRssThemeSync.createSync({read:async()=>{if(++n===1)throw Error('fixture');return {revision:2};},revision:()=>0,active:()=>true,apply:()=>applied++,onError:()=>errors++});
 await retry.check();await retry.check();assert.equal(errors,1);assert.equal(applied,1);
});
