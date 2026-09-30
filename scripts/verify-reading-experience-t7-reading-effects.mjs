// Final APK M14: native reading typography, anchor, restart and keyboard mark-read semantics.
// ANDROID_SERIAL=emulator-5584 node scripts/verify-reading-experience-t7-reading-effects.mjs OUT
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {copyFileSync,mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import {resolve} from 'node:path';

const serial=process.env.ANDROID_SERIAL;assert.equal(serial,'emulator-5584');
const out=resolve(process.argv[2]);mkdirSync(out,{recursive:true});
const adb=(...args)=>execFileSync('/usr/lib/android-sdk/platform-tools/adb',['-s',serial,...args],{timeout:30000,maxBuffer:20*1024*1024});
const say=(...args)=>adb(...args).toString().trim(),wait=ms=>new Promise(ok=>setTimeout(ok,ms));
assert.match(say('emu','avd','name'),/RustRssT5/);
const hash=file=>createHash('sha256').update(readFileSync(file)).digest('hex');
const apkSha256=hash('src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk');
const fixture='/tmp/rustrss-t7-reading.sqlite';copyFileSync('/tmp/rustrss-t4-fixture.sqlite',fixture);
execFileSync('python3',['-c',`import sqlite3,sys
c=sqlite3.connect(sys.argv[1]);c.execute("UPDATE entries SET content_html=content_html || '<pre><code>let reading = 42; // native M14</code></pre>' WHERE id=1");c.commit();c.close()`,fixture]);
const fixtureSha256=hash(fixture),shot=name=>writeFileSync(`${out}/${name}.png`,adb('exec-out','screencap','-p'));
adb('push',fixture,'/data/local/tmp/rustrss-t7-reading.sqlite');
adb('shell','am','force-stop','tech.expoli.rustrss');
adb('shell','run-as','tech.expoli.rustrss','cp','/data/local/tmp/rustrss-t7-reading.sqlite','rustrss/rustrss.sqlite');
adb('shell','run-as','tech.expoli.rustrss','rm','-f','rustrss/rustrss.sqlite-wal','rustrss/rustrss.sqlite-shm');
async function attach(){
  adb('shell','am','force-stop','tech.expoli.rustrss');adb('shell','am','start','-n','tech.expoli.rustrss/.MainActivity');
  let pid='';for(let i=0;i<50;i++){try{pid=say('shell','pidof','tech.expoli.rustrss').split(' ')[0]}catch{}if(pid)break;await wait(200)}assert(pid);
  adb('forward','tcp:9237',`localabstract:webview_devtools_remote_${pid}`);
  let target;for(let i=0;i<50;i++){try{target=(await(await fetch('http://127.0.0.1:9237/json')).json()).find(t=>t.type==='page'&&JSON.parse(t.description||'{}').attached);if(target)break}catch{}await wait(200)}assert(target);
  const ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((ok,fail)=>{ws.onopen=ok;ws.onerror=fail});
  let id=0;const pending=new Map();ws.onmessage=e=>{const m=JSON.parse(e.data),p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(m.error):p.ok(m.result)};
  const js=expr=>new Promise((ok,fail)=>{const n=++id,timer=setTimeout(()=>{pending.delete(n);fail(Error('CDP timeout'))},15000);pending.set(n,{ok:r=>r.exceptionDetails?fail(Error(JSON.stringify(r.exceptionDetails))):ok(r.result.value),fail,timer});ws.send(JSON.stringify({id:n,method:'Runtime.evaluate',params:{expression:expr,returnByValue:true,awaitPromise:true}}))});
  const key=(type,key,code,virtualKeyCode)=>new Promise((ok,fail)=>{const n=++id,timer=setTimeout(()=>{pending.delete(n);fail(Error('CDP timeout'))},15000);pending.set(n,{ok,fail,timer});ws.send(JSON.stringify({id:n,method:'Input.dispatchKeyEvent',params:{type,key,code,windowsVirtualKeyCode:virtualKeyCode,nativeVirtualKeyCode:virtualKeyCode}}))});
  const until=async expr=>{for(let i=0;i<100;i++){try{if(await js(expr))return}catch{}await wait(100)}throw Error(`Timed out ${expr}`)};
  const core=(cmd,args={})=>js(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
  return {ws,js,until,core,key,pid};
}
const measure=()=>`(()=>{const reader=document.querySelector('#reader'),p=reader.querySelector('.article p'),code=reader.querySelector('.article code'),anchor=[...reader.querySelectorAll('.article > *')].find(n=>n.getBoundingClientRect().bottom>reader.getBoundingClientRect().top+8);return {scrollTop:reader.scrollTop,anchorText:anchor?.textContent?.slice(0,36),anchorY:anchor?.getBoundingClientRect().top,readFamily:getComputedStyle(p).fontFamily,monoFamily:getComputedStyle(code).fontFamily,monoSize:getComputedStyle(code).fontSize,readerWidth:reader.getBoundingClientRect().width,viewport:visualViewport.width,overflow:document.documentElement.scrollWidth>visualViewport.width+1}})()`;
const set=(selector,value)=>`(()=>{const n=document.querySelector(${JSON.stringify(selector)});n.value=${JSON.stringify(value)};n.dispatchEvent(new Event('change',{bubbles:true}));return true})()`;
let p;
try{
  p=await attach();await p.until(`!!document.querySelector('#entries li[data-id]')`);
  await p.js(`document.querySelector('[data-mview="all"]').click();true`);
  await p.until(`document.querySelector('#views li.active')?.dataset.kind==='all'`);
  await p.until(`!!document.querySelector('#entries li[data-id="1"]')`);
  await p.js(`document.querySelector('#entries li[data-id="1"] .title').click();true`);
  await p.until(`document.body.dataset.mpage==='reader' && !!document.querySelector('#reader .article code')`);
  await p.js(`document.querySelector('#reader').scrollTop=420;true`);await wait(200);
  const before=await p.js(measure());shot('reading-before');
  await p.js(`document.querySelector('#act-aa').click();document.querySelector('#aa-editor .theme-advanced').open=true;true`);
  await p.until(`!!document.querySelector('#aa-editor [data-theme-field="typography.read_family"]')?.getClientRects().length`);
  const fields={read:await p.js(`!!document.querySelector('#aa-editor [data-theme-field="typography.read_family"]')?.getClientRects().length`),mono:await p.js(`!!document.querySelector('#aa-editor [data-theme-field="typography.mono_family"]')?.getClientRects().length`),monoSize:await p.js(`!!document.querySelector('#aa-editor [data-theme-field="typography.mono_size"]')?.getClientRects().length`)};
  assert(Object.values(fields).every(Boolean));
  await p.js(set('#aa-editor [data-theme-field="typography.read_family"]','serif'));
  await p.js(set('#aa-editor [data-theme-field="typography.mono_family"]','monospace'));
  await p.js(set('#aa-editor [data-theme-field="typography.mono_size"]',20));
  const rev=(await p.core('get_ui_settings')).theme_snapshot.config.revision;
  await p.js(`document.querySelector('#aa-editor .theme-editor-actions button:first-child').click();true`);
  await p.until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.theme_snapshot.config.revision===${rev+1})`);
  const after=await p.js(measure()),saved=(await p.core('get_ui_settings')).theme_snapshot;shot('reading-saved');
  assert(after.readFamily.includes('serif'));assert(after.monoFamily.includes('monospace'));assert.equal(after.monoSize,'20px');assert.equal(after.anchorText,before.anchorText);assert(Math.abs(after.anchorY-before.anchorY)<3,{before,after});assert(!after.overflow);
  p.ws.close();p=null;
  p=await attach();await p.until(`!!document.querySelector('#entries li[data-id]')`);
  await p.js(`document.querySelector('[data-mview="all"]').click();true`);
  await p.until(`document.querySelector('#views li.active')?.dataset.kind==='all'`);
  await p.until(`!!document.querySelector('#entries li[data-id="1"]')`);
  await p.js(`document.querySelector('#entries li[data-id="1"] .title').click();true`);
  await p.until(`document.body.dataset.mpage==='reader' && !!document.querySelector('#reader .article code')`);
  const retained=await p.js(measure()),restart=(await p.core('get_ui_settings')).theme_snapshot;shot('reading-restarted');
  assert(retained.readFamily.includes('serif'));assert(retained.monoFamily.includes('monospace'));assert.equal(retained.monoSize,'20px');assert.deepEqual(restart.light.typography.read_family,['serif']);assert.equal(restart.light.typography.mono_size,20);
  // The setting applies to j/k and arrow navigation; list taps intentionally always mark read.
  await p.core('set_read',{ids:Array.from({length:30},(_,i)=>i+1),read:false});
  await p.core('set_mark_read_on_navigate',{enabled:false});p.ws.close();p=null;
  p=await attach();await p.until(`!!document.querySelector('#entries li[data-id]')`);
  await p.js(`document.querySelector('[data-mview="all"]').click();document.querySelector('#entries').focus();true`);
  await p.until(`document.querySelector('#views li.active')?.dataset.kind==='all'`);
  await p.js(`window.__t7Keys=[];document.addEventListener('keydown',e=>window.__t7Keys.push({key:e.key,trusted:e.isTrusted}),true);true`);
  await p.key('keyDown','j','KeyJ',74);await p.key('keyUp','j','KeyJ',74);
  await p.until(`document.body.dataset.mpage==='reader' && !!document.querySelector('#entries li.active')`);
  const offId=Number(await p.js(`document.querySelector('#entries li.active').dataset.id`));
  const offKeyEvent=await p.js(`window.__t7Keys.at(-1)`);assert.equal(offKeyEvent?.trusted,true);
  const keyOff=await p.core('get_entry',{id:offId});assert.equal(keyOff.read,false);
  shot('mark-read-keyboard-off');
  await p.core('set_mark_read_on_navigate',{enabled:true});p.ws.close();p=null;
  p=await attach();await p.until(`!!document.querySelector('#entries li[data-id]')`);
  await p.js(`document.querySelector('[data-mview="all"]').click();document.querySelector('#entries').focus();true`);
  await p.until(`document.querySelector('#views li.active')?.dataset.kind==='all'`);
  await p.js(`window.__t7Keys=[];document.addEventListener('keydown',e=>window.__t7Keys.push({key:e.key,trusted:e.isTrusted}),true);true`);
  await p.key('keyDown','j','KeyJ',74);await p.key('keyUp','j','KeyJ',74);
  await p.until(`document.body.dataset.mpage==='reader' && !!document.querySelector('#entries li.active')`);
  const onId=Number(await p.js(`document.querySelector('#entries li.active').dataset.id`));
  const onKeyEvent=await p.js(`window.__t7Keys.at(-1)`);assert.equal(onKeyEvent?.trusted,true);
  await p.until(`window.__TAURI__.core.invoke('get_entry',{id:${onId}}).then(e=>e.read===true)`);
  const keyOn=await p.core('get_entry',{id:onId});shot('mark-read-keyboard-on');assert.equal(keyOn.read,true);
  const result={serial,avd:'RustRssT5',apkSha256,fixtureSha256,fields,before,after,savedRevision:saved.config.revision,retained,restartRevision:restart.config.revision,keyboard:{input:'CDP Input.dispatchKeyEvent',off:{entryId:offId,readAfter:keyOff.read,event:offKeyEvent},on:{entryId:onId,readAfter:keyOn.read,event:onKeyEvent}},passed:true};
  writeFileSync(`${out}/reading-effects.json`,JSON.stringify(result,null,2)+'\n');
  console.log(JSON.stringify({apkSha256,anchorDelta:after.anchorY-before.anchorY,retained:retained.monoSize,keyboard:result.keyboard}));
}finally{p?.ws.close()}
