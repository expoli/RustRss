// Final APK M13: UI family, density and thumbnail network effect across restart.
// ANDROID_SERIAL=emulator-5584 node scripts/verify-reading-experience-t7-appearance-effects.mjs OUT
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createServer} from 'node:http';
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
const image=Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/EqkAAAAASUVORK5CYII=','base64');
const requests=[];const server=createServer((req,res)=>{requests.push({path:req.url,time:Date.now()});res.writeHead(200,{'content-type':'image/png','cache-control':'no-store','content-length':image.length});res.end(image)});
await new Promise(ok=>server.listen(0,'0.0.0.0',ok));
const fixture='/tmp/rustrss-t7-appearance.sqlite';copyFileSync('/tmp/rustrss-t4-fixture.sqlite',fixture);
const imageUrl=`http://10.0.2.2:${server.address().port}/image.png`;
execFileSync('python3',['-c','import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute("UPDATE entries SET thumbnail_url=NULL"); c.execute("UPDATE entries SET thumbnail_url=? WHERE id=1",(sys.argv[2],)); c.commit(); c.close()',fixture,imageUrl]);
const fixtureSha256=hash(fixture),shot=name=>writeFileSync(`${out}/${name}.png`,adb('exec-out','screencap','-p'));
adb('push',fixture,'/data/local/tmp/rustrss-t7-appearance.sqlite');
adb('shell','am','force-stop','tech.expoli.rustrss');
adb('shell','run-as','tech.expoli.rustrss','cp','/data/local/tmp/rustrss-t7-appearance.sqlite','rustrss/rustrss.sqlite');
adb('shell','run-as','tech.expoli.rustrss','rm','-f','rustrss/rustrss.sqlite-wal','rustrss/rustrss.sqlite-shm');
async function attach(){
  adb('shell','am','force-stop','tech.expoli.rustrss');adb('shell','am','start','-n','tech.expoli.rustrss/.MainActivity');
  let pid='';for(let i=0;i<50;i++){try{pid=say('shell','pidof','tech.expoli.rustrss').split(' ')[0]}catch{}if(pid)break;await wait(200)}assert(pid);
  adb('forward','tcp:9237',`localabstract:webview_devtools_remote_${pid}`);
  let target;for(let i=0;i<50;i++){try{target=(await(await fetch('http://127.0.0.1:9237/json')).json()).find(t=>t.type==='page'&&JSON.parse(t.description||'{}').attached);if(target)break}catch{}await wait(200)}assert(target);
  const ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((ok,fail)=>{ws.onopen=ok;ws.onerror=fail});
  let id=0;const pending=new Map();ws.onmessage=e=>{const m=JSON.parse(e.data),p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(m.error):p.ok(m.result)};
  const js=expr=>new Promise((ok,fail)=>{const n=++id,timer=setTimeout(()=>{pending.delete(n);fail(Error('CDP timeout'))},15000);pending.set(n,{ok:r=>r.exceptionDetails?fail(Error(JSON.stringify(r.exceptionDetails))):ok(r.result.value),fail,timer});ws.send(JSON.stringify({id:n,method:'Runtime.evaluate',params:{expression:expr,returnByValue:true,awaitPromise:true}}))});
  const until=async expr=>{for(let i=0;i<100;i++){try{if(await js(expr))return}catch{}await wait(100)}throw Error(`Timed out ${expr}`)};
  const core=(cmd,args={})=>js(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
  return {ws,js,until,core,pid};
}
const measure=()=>`(()=>{const n=document.querySelector('#entries li[data-id="1"]'),r=n?.getBoundingClientRect(),title=n?.querySelector('.title');return {font:getComputedStyle(title).fontFamily,rowHeight:r?.height,padding:document.documentElement.style.getPropertyValue('--row-padding'),fontToken:document.documentElement.style.getPropertyValue('--font-ui'),thumbnails:document.documentElement.dataset.thumbnails,imageVisible:!!document.querySelector('.entry-thumbnail')?.getClientRects().length,scrollWidth:document.documentElement.scrollWidth,viewport:visualViewport.width}})()`;
const set=(selector,value)=>`(()=>{const n=document.querySelector(${JSON.stringify(selector)});if(n.type==='checkbox')n.checked=${JSON.stringify(value)};else n.value=${JSON.stringify(value)};n.dispatchEvent(new Event('change',{bubbles:true}));return true})()`;
async function save(p){const before=(await p.core('get_ui_settings')).theme_snapshot.config.revision;await p.js(`document.querySelector('#appearance-editor .theme-editor-actions button:first-child').click();true`);await p.until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.theme_snapshot.config.revision===${before+1})`);return (await p.core('get_ui_settings')).theme_snapshot}
let p;
try{
  p=await attach();await p.until(`!!document.querySelector('#entries li[data-id="1"]')`);
  const baseline=await p.js(measure());shot('appearance-before');
  assert.equal(baseline.padding,'9px');assert.equal(baseline.thumbnails,'true');
  await p.js(`document.querySelector('#m-nav [data-mpage-btn="settings"]').click();document.querySelector('#tab-appearance').click();document.querySelector('#appearance-editor .theme-advanced').open=true;true`);
  await p.until(`!!document.querySelector('#appearance-editor [data-theme-field="typography.ui_family"]')?.getClientRects().length`);
  const fields={family:await p.js(`!!document.querySelector('#appearance-editor [data-theme-field="typography.ui_family"]')?.getClientRects().length`),density:await p.js(`!!document.querySelector('#appearance-editor [data-theme-field="list.density"]')?.getClientRects().length`),thumbnail:await p.js(`!!document.querySelector('#appearance-editor [data-theme-field="list.thumbnail"]')?.getClientRects().length`)};
  assert(Object.values(fields).every(Boolean));
  await p.js(set('#appearance-editor [data-theme-field="typography.ui_family"]','serif'));
  await p.js(set('#appearance-editor [data-theme-field="list.density"]','compact'));
  await p.js(set('#appearance-editor [data-theme-field="list.thumbnail"]',false));
  const saved=await save(p);
  assert.deepEqual(saved.light.typography.ui_family,['serif']);assert.equal(saved.light.list.density,'compact');assert.equal(saved.light.list.thumbnail,false);
  await p.js(`document.querySelector('#m-nav [data-mpage-btn="articles"]').click();true`);await p.until(`!!document.querySelector('#entries li[data-id="1"]')`);
  const changed=await p.js(measure());shot('appearance-saved');
  assert.equal(changed.padding,'5px');assert.equal(changed.thumbnails,'false');assert(changed.font.includes('serif'));assert(changed.rowHeight<baseline.rowHeight);
  p.ws.close();p=null;requests.length=0;
  p=await attach();await p.until(`!!document.querySelector('#entries li[data-id="1"]')`);await wait(500);
  const retained=await p.js(measure());const restartSettings=(await p.core('get_ui_settings')).theme_snapshot;shot('appearance-restarted-off');
  const offRequests=requests.filter(r=>r.path==='/image.png').length;
  assert.equal(offRequests,0,JSON.stringify(requests));assert.equal(retained.padding,'5px');assert.equal(retained.thumbnails,'false');assert(retained.font.includes('serif'));
  assert.equal(restartSettings.light.list.thumbnail,false);
  await p.js(`document.querySelector('#m-nav [data-mpage-btn="settings"]').click();document.querySelector('#tab-appearance').click();document.querySelector('#appearance-editor .theme-advanced').open=true;true`);
  await p.js(set('#appearance-editor [data-theme-field="list.thumbnail"]',true));
  const restored=await save(p);assert.equal(restored.light.list.thumbnail,true);
  p.ws.close();p=null;requests.length=0;
  p=await attach();await p.until(`!!document.querySelector('#entries li[data-id="1"]')`);await wait(500);
  const onRequests=requests.filter(r=>r.path==='/image.png').length;
  const on=await p.js(measure());shot('appearance-restarted-on');
  assert(onRequests>=1,JSON.stringify(requests));assert.equal(on.thumbnails,'true');
  const result={serial,avd:'RustRssT5',apkSha256,fixtureSha256,imageUrl,fields,baseline,saved:{revision:saved.config.revision,uiFamily:saved.light.typography.ui_family,density:saved.light.list.density,thumbnail:saved.light.list.thumbnail},changed,retained,restartRevision:restartSettings.config.revision,offRequests,onRequests,on,passed:true};
  writeFileSync(`${out}/appearance-effects.json`,JSON.stringify(result,null,2)+'\n');
  console.log(JSON.stringify({offRequests,onRequests,rowHeights:[baseline.rowHeight,changed.rowHeight],font:retained.font,apkSha256}));
}finally{p?.ws.close();server.close()}
