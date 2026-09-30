// Same-fixture native APK captures for the pre-change and final builds.
// ANDROID_SERIAL=emulator-5584 BASELINE_APK=/abs/path.apk T7_CAPTURE_LABEL=before T7_CAPTURE_COMMIT=bc680d5 node scripts/verify-reading-experience-t7-android-baseline.mjs OUT
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import {resolve} from 'node:path';

const serial=process.env.ANDROID_SERIAL,apk=process.env.BASELINE_APK;
assert.equal(serial,'emulator-5584');assert(apk);
const label=process.env.T7_CAPTURE_LABEL||'before',commit=process.env.T7_CAPTURE_COMMIT||'bc680d5';
assert(['before','after'].includes(label));
const out=resolve(process.argv[2]);mkdirSync(out,{recursive:true});
const adb=(...args)=>execFileSync('/usr/lib/android-sdk/platform-tools/adb',['-s',serial,...args],{timeout:30000,maxBuffer:20*1024*1024});
const say=(...args)=>adb(...args).toString().trim();
assert.match(say('emu','avd','name'),/RustRssT5/);
const fixture='/tmp/rustrss-t4-fixture.sqlite';
const hash=file=>createHash('sha256').update(readFileSync(file)).digest('hex');
const wait=ms=>new Promise(ok=>setTimeout(ok,ms));
adb('install','-r',apk);
adb('shell','pm','clear','tech.expoli.rustrss');
adb('push',fixture,'/data/local/tmp/rustrss-t7-baseline-fixture.sqlite');
adb('shell','am','start','-n','tech.expoli.rustrss/.MainActivity');
await wait(1500);
adb('shell','am','force-stop','tech.expoli.rustrss');
adb('shell','run-as','tech.expoli.rustrss','mkdir','-p','rustrss');
adb('shell','run-as','tech.expoli.rustrss','cp','/data/local/tmp/rustrss-t7-baseline-fixture.sqlite','rustrss/rustrss.sqlite');
adb('shell','run-as','tech.expoli.rustrss','rm','-f','rustrss/rustrss.sqlite-wal','rustrss/rustrss.sqlite-shm');
adb('shell','am','start','-n','tech.expoli.rustrss/.MainActivity');
let pid='';for(let i=0;i<50;i++){try{pid=say('shell','pidof','tech.expoli.rustrss').split(' ')[0]}catch{}if(pid)break;await wait(200)}
assert(pid);
adb('forward','tcp:9237',`localabstract:webview_devtools_remote_${pid}`);
let target;for(let i=0;i<50;i++){try{target=(await(await fetch('http://127.0.0.1:9237/json')).json()).find(t=>t.type==='page'&&JSON.parse(t.description||'{}').attached);if(target)break}catch{}await wait(200)}
assert(target);
const ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((ok,fail)=>{ws.onopen=ok;ws.onerror=fail});
let id=0;const pending=new Map();
ws.onmessage=e=>{const m=JSON.parse(e.data),slot=pending.get(m.id);if(!slot)return;clearTimeout(slot.timer);pending.delete(m.id);m.error?slot.fail(m.error):slot.ok(m.result)};
const js=expression=>new Promise((ok,fail)=>{const next=++id,timer=setTimeout(()=>{pending.delete(next);fail(Error('CDP timeout'))},15000);pending.set(next,{ok:result=>{if(result.exceptionDetails)fail(Error(JSON.stringify(result.exceptionDetails)));else ok(result.result.value)},fail,timer});ws.send(JSON.stringify({id:next,method:'Runtime.evaluate',params:{expression,returnByValue:true,awaitPromise:true}}))});
const until=async expr=>{for(let i=0;i<100;i++){try{if(await js(expr))return}catch{}await wait(100)}throw Error(`Timed out ${expr}`)};
const shot=name=>writeFileSync(`${out}/${name}.png`,adb('exec-out','screencap','-p'));
const state=()=>js(`({page:document.body.dataset.mpage,css:[visualViewport.width,visualViewport.height],dpr:devicePixelRatio,scrollWidth:document.documentElement.scrollWidth,firstTitle:document.querySelector('#entries li[data-id] .title')?.textContent,firstReaderParagraph:document.querySelector('.article p')?.textContent?.slice(0,80)})`);
try{
  await until(`!!document.querySelector('#entries li[data-id] .title')`);
  const list=await state();assert(list.scrollWidth<=list.css[0]+1);shot(`${label}-articles`);
  await js(`document.querySelector('#entries li[data-id] .title').click();true`);
  await until(`document.body.dataset.mpage==='reader'`);
  const reader=await state();assert(reader.scrollWidth<=reader.css[0]+1);shot(`${label}-reader`);
  await js(`document.querySelector('[data-mpage-btn="settings"]').click();true`);
  await until(`document.body.dataset.mpage==='settings'`);
  const settings=await state();shot(`${label}-settings`);
  const record={commit,label,serial,avd:'RustRssT5',apkSha256:hash(apk),fixtureSha256:hash(fixture),physicalSize:say('shell','wm','size'),density:say('shell','wm','density'),list,reader,settings,passed:true};
  writeFileSync(`${out}/android-${label}.json`,JSON.stringify(record,null,2)+'\n');
  console.log(JSON.stringify({apkSha256:record.apkSha256,fixtureSha256:record.fixtureSha256,css:list.css}));
}finally{ws.close()}
