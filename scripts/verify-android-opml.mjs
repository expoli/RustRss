// Use an isolated emulator with the rebuilt debug APK and CDP forwarded to 9227.
// ANDROID_SERIAL=emulator-5580 node scripts/verify-android-opml.mjs <evidence-dir>
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {mkdirSync, writeFileSync} from 'node:fs';
import {resolve} from 'node:path';
const serial=process.env.ANDROID_SERIAL; assert(serial,'Set ANDROID_SERIAL to an isolated emulator');
const adb=process.env.ADB||'/usr/lib/android-sdk/platform-tools/adb';
const out=resolve(process.argv[2]||'/tmp/rustrss-opml-picker'); mkdirSync(out,{recursive:true});
const run=(...a)=>execFileSync(adb,['-s',serial,...a],{timeout:20000});
const wait=ms=>new Promise(r=>setTimeout(r,ms));
const tabs=await(await fetch(process.env.CDP_URL||'http://127.0.0.1:9227/json')).json();
const ws=new WebSocket(tabs.find(t=>t.type==='page').webSocketDebuggerUrl);
await new Promise((r,j)=>{ws.onopen=r;ws.onerror=j});
let seq=0;const pending=new Map();
ws.onmessage=e=>{const m=JSON.parse(e.data),p=pending.get(m.id);if(p){pending.delete(m.id);m.error?p.reject(m.error):p.resolve(m.result)}};
const evaluate=expression=>new Promise((resolve,reject)=>{const id=++seq;pending.set(id,{resolve:r=>{r.exceptionDetails?reject(r.exceptionDetails):resolve(r.result.value)},reject});ws.send(JSON.stringify({id,method:'Runtime.evaluate',params:{expression,returnByValue:true,awaitPromise:true}}))});
function tree(){const dumped=run('shell','uiautomator','dump','/data/local/tmp/rustrss-opml-picker.xml').toString();return dumped.includes('dumped to:')?run('shell','cat','/data/local/tmp/rustrss-opml-picker.xml').toString():''}
const nodes=xml=>[...xml.matchAll(/<node\b[^>]*>/g)].map(m=>m[0]);
function touch(node){const[,x1,y1,x2,y2]=node.match(/bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/).map(Number);run('shell','input','tap',String(Math.round((x1+x2)/2)),String(Math.round((y1+y2)/2)))}
function screen(name){writeFileSync(`${out}/${name}.png`,run('exec-out','screencap','-p'))}
const feeds=()=>evaluate(`window.__TAURI__.core.invoke('list_feeds').then(f=>f.map(x=>x.url).sort())`);
const checks=[];
async function openPicker(){
 await evaluate(`window.__opmlProbe={pending:true};window.__TAURI__.core.invoke('import_opml').then(result=>window.__opmlProbe={result},error=>window.__opmlProbe={error:String(error)});true`);
 for(let i=0;i<12;i++){
  const xml=tree();
  if(xml.includes('com.google.android.documentsui')){
   // A picker can start in Recent; always use the actual Downloads root.
   const download=nodes(xml).find(n=>n.includes('resource-id="android:id/title"')&&(n.includes('text="Downloads"')||n.includes('text="下载"')));
   if(download){touch(download);await wait(300);return}
   const menu=nodes(xml).find(n=>n.includes('content-desc="Show roots"')||n.includes('content-desc="显示根目录"'));
   if(menu)touch(menu);
  }
  await wait(200);
 }
 assert.fail('Android document picker did not open');
}
async function outcome(){
 for(let i=0;i<100;i++){const r=await evaluate('window.__opmlProbe');if(!r.pending)return r;await wait(100)}
 assert.fail('Picker result did not reach the import command');
}
async function pick(name,label){
 await openPicker();let node;
 for(let i=0;i<5;i++){node=nodes(tree()).find(n=>n.includes(`text="${name}"`));if(node)break;await wait(200)}
 assert(node,`Missing Downloads fixture ${name}`);screen(label+'-picker');
 assert(node.includes('enabled="true"'),`${name} is disabled by picker MIME filtering: ${node}`);
 touch(node);const result=await outcome();screen(label+'-result');checks.push({label,...result});return result;
}
try{
 let ready=false;
 for(let i=0;i<100;i++){ready=await evaluate(`!!window.__TAURI__ && !!document.querySelector('#act-import-opml')?.onclick`);if(ready)break;await wait(100)}
 assert(ready,'App scripts are not ready');
 const prefix=`https://opml-picker.example.invalid/${Date.now()}/`;
 const opml=suffix=>`<?xml version="1.0"?><opml version="2.0"><body><outline text="Picker ${suffix}" type="rss" xmlUrl="${prefix}${suffix}"/></body></opml>`;
 for(const[name,content]of [['RustRss-generic.opml',opml('generic')],['RustRss-xml.xml',opml('xml')],['RustRss-invalid.opml','<html><body>Not OPML</body></html>'],['RustRss-malformed.opml','<opml><body><outline']]){
  writeFileSync(`${out}/${name}`,content);run('push',`${out}/${name}`,`/sdcard/Download/${name}`);
 }
 const initial=await feeds();assert(!initial.some(u=>u.startsWith(prefix)),'Use a clean isolated app fixture');
 let r=await pick('RustRss-generic.opml','01-generic');assert.equal(r.result?.feeds_added,1);assert((await feeds()).includes(prefix+'generic'));
 r=await pick('RustRss-generic.opml','02-duplicate');assert.equal(r.result?.feeds_added,0);assert.equal(r.result?.feeds_skipped,1);
 r=await pick('RustRss-xml.xml','03-xml');assert.equal(r.result?.feeds_added,1);assert((await feeds()).includes(prefix+'xml'));
 const imported=await feeds();
 for(const[name,label]of [['RustRss-invalid.opml','04-invalid'],['RustRss-malformed.opml','05-malformed']]){r=await pick(name,label);assert(r.error,`${name} unexpectedly imported`);assert(r.error.includes(label==='04-invalid'?'没有 <opml>':'位置'),r.error);assert.deepEqual(await feeds(),imported)}
 await openPicker();
 // Android Back may first leave Downloads for Recent; cancel the picker itself.
 for(let i=0;i<6;i++){
  const top=run('shell','dumpsys','activity','activities').toString().split('\n').find(l=>l.includes('topResumedActivity'))||'';
  if(!top.includes('documentsui'))break;
  run('shell','input','keyevent','4');await wait(250);
 }
 r=await outcome();assert.equal(r.result,null);assert.deepEqual(await feeds(),imported);checks.push({label:'06-cancel',...r});
 writeFileSync(`${out}/results.json`,JSON.stringify({serial,initialFeeds:initial.length,finalFeeds:imported.length,checks},null,2)+'\n');console.log(JSON.stringify({out,checks:checks.length}));
}finally{ws.close()}
