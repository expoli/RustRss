// Installed APK phone geometry with real AVD display, navigation, orientation and IME state.
// ANDROID_SERIAL=emulator-5584 node scripts/verify-reading-experience-t7-phone-grid.mjs OUT
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import {resolve} from 'node:path';

const serial=process.env.ANDROID_SERIAL;assert.equal(serial,'emulator-5584');
const out=resolve(process.argv[2]);mkdirSync(out,{recursive:true});
const adb=(...args)=>execFileSync('/usr/lib/android-sdk/platform-tools/adb',['-s',serial,...args],{timeout:30000,maxBuffer:20*1024*1024});
const say=(...args)=>adb(...args).toString().trim(),pause=ms=>new Promise(ok=>setTimeout(ok,ms));
assert.match(say('emu','avd','name'),/RustRssT5/);
const apk='src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk';
const apkSha256=createHash('sha256').update(readFileSync(apk)).digest('hex');
const original={size:say('shell','wm','size'),density:say('shell','wm','density'),rotation:say('shell','settings','get','system','user_rotation'),auto:say('shell','settings','get','system','accelerometer_rotation'),font:say('shell','settings','get','system','font_scale'),nav:say('shell','settings','get','secure','navigation_mode')};
const results=[];
const shot=name=>writeFileSync(`${out}/${name}.png`,adb('exec-out','screencap','-p'));
const setNav=mode=>adb('shell','cmd','overlay','enable-exclusive','--user','0','--category',`com.android.internal.systemui.navbar.${mode}`);
async function attach(){
  adb('shell','am','force-stop','tech.expoli.rustrss');
  adb('shell','am','start','-n','tech.expoli.rustrss/.MainActivity');
  let pid='';for(let i=0;i<50;i++){try{pid=say('shell','pidof','tech.expoli.rustrss').split(' ')[0]}catch{}if(pid)break;await pause(200)}assert(pid);
  adb('forward','tcp:9237',`localabstract:webview_devtools_remote_${pid}`);
  let target;for(let i=0;i<50;i++){try{target=(await(await fetch('http://127.0.0.1:9237/json')).json()).find(t=>t.type==='page'&&JSON.parse(t.description||'{}').attached);if(target)break}catch{}await pause(200)}assert(target);
  const ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((ok,fail)=>{ws.onopen=ok;ws.onerror=fail});
  let id=0;const pending=new Map();ws.onmessage=e=>{const m=JSON.parse(e.data),slot=pending.get(m.id);if(!slot)return;clearTimeout(slot.timer);pending.delete(m.id);m.error?slot.fail(m.error):slot.ok(m.result)};
  const js=expression=>new Promise((ok,fail)=>{const next=++id,timer=setTimeout(()=>{pending.delete(next);fail(Error('CDP timeout'))},15000);pending.set(next,{ok:r=>r.exceptionDetails?fail(Error(JSON.stringify(r.exceptionDetails))):ok(r.result.value),fail,timer});ws.send(JSON.stringify({id:next,method:'Runtime.evaluate',params:{expression,returnByValue:true,awaitPromise:true}}))});
  const until=async expr=>{for(let i=0;i<100;i++){try{if(await js(expr))return}catch{}await pause(100)}throw Error(`Timed out ${expr}`)};
  const tap=async selector=>{const rect=await js(`(()=>{const n=document.querySelector(${JSON.stringify(selector)});n?.scrollIntoView({block:'center'});if(!n)return null;const r=n.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;return{x,y,w:r.width,h:r.height,hit:document.elementFromPoint(x,y)?.closest(${JSON.stringify(selector)})===n,dpr:devicePixelRatio}})()`);assert(rect?.w>0&&rect.h>0&&rect.hit,JSON.stringify({selector,rect}));adb('shell','input','tap',String(Math.round(rect.x*rect.dpr)),String(Math.round(JSON.parse(target.description).screenY+rect.y*rect.dpr)));await pause(300);return rect};
  return {ws,js,until,tap,pid,target};
}
const geometry=()=>`(()=>{const rect=n=>{if(!n)return null;const r=n.getBoundingClientRect();return{x:r.x,y:r.y,w:r.width,h:r.height,right:r.right,bottom:r.bottom,visible:!!n.getClientRects().length}};const box=s=>rect(document.querySelector(s));const nav=[...document.querySelectorAll('#m-nav button')].filter(n=>n.getClientRects().length).map(n=>({name:n.getAttribute('aria-label')||n.textContent.trim(),...rect(n)}));return{page:document.body.dataset.mpage,viewport:{w:visualViewport.width,h:visualViewport.height},window:{w:innerWidth,h:innerHeight},dpr:devicePixelRatio,scrollWidth:document.documentElement.scrollWidth,nav,firstTitle:box('#entries li[data-id] .title'),firstParagraph:box('.article p'),readerBack:box('#m-reader-back'),readerActions:['#act-aa','#act-star','#act-later','#act-more'].map(s=>({id:s,...box(s)})),settings:box('#btn-settings'),mcp:box('#tab-mcp'),windowMin:box('#btn-win-min')}})()`;
async function scenario(name,size,nav,rotation){
  adb('shell','wm','size',size);
  setNav(nav);
  adb('shell','settings','put','system','accelerometer_rotation','0');
  adb('shell','settings','put','system','user_rotation',String(rotation));
  await pause(900);
  const p=await attach();
  try{
    await p.until(`!!document.querySelector('#entries li[data-id] .title')`);
    const list=await p.js(geometry());shot(`${name}-list`);
    assert(list.scrollWidth<=list.viewport.w+1,JSON.stringify(list));
    assert.equal(list.nav.length,4);
    assert(list.nav.every(n=>n.w>=48&&n.h>=48&&n.x>=0&&n.right<=list.viewport.w+1&&n.bottom<=list.viewport.h+1),JSON.stringify(list.nav));
    assert(!list.windowMin.visible&&!list.mcp.visible);
    await p.tap('#entries li[data-id] .title');await p.until(`document.body.dataset.mpage==='reader'`);
    const reader=await p.js(geometry());shot(`${name}-reader`);
    assert(reader.scrollWidth<=reader.viewport.w+1,JSON.stringify(reader));
    assert(reader.readerBack?.visible&&reader.readerBack.w>=48&&reader.readerBack.h>=48);
    assert(reader.readerActions.every(n=>n.visible&&n.w>=48&&n.h>=48&&n.right<=reader.viewport.w+1),JSON.stringify(reader.readerActions));
    adb('shell','input','keyevent','KEYCODE_BACK');await p.until(`document.body.dataset.mpage==='articles'`);
    const afterBack=await p.js(geometry());
    const orientation=say('shell','dumpsys','window').match(/mRotation=(\d+)/)?.[1]||null;
    const actualNav=say('shell','settings','get','secure','navigation_mode');
    const result={name,size,nav,rotation,actualRotation:orientation,actualNav,physical:say('shell','wm','size'),density:say('shell','wm','density'),screen:JSON.parse(p.target.description),pid:p.pid,list,reader,afterBack};
    results.push(result);
    if(rotation===1) assert.equal(orientation,'1',JSON.stringify(result));
    if(nav==='threebutton') assert.equal(actualNav,'0');else assert.equal(actualNav,'2');
    return p;
  }finally{p.ws.close()}
}
try{
  await scenario('360-gesture-portrait','945x2100','gestural',0);
  await scenario('412-threebutton-portrait','1080x2400','threebutton',0);
  await scenario('360-gesture-landscape','945x2100','gestural',1);
  adb('shell','settings','put','system','user_rotation','0');await pause(800);
  const p=await attach();
  try{
    await p.until(`!!document.querySelector('#entries li[data-id] .title')`);
    const before=await p.js(geometry());await p.tap('#btn-search-toggle');await p.until(`!!document.querySelector('#search')?.getClientRects().length`);await p.tap('#search');await pause(700);
    const ime=await p.js(`({viewport:{w:visualViewport.width,h:visualViewport.height},active:document.activeElement?.id,scrollWidth:document.documentElement.scrollWidth,nav:[...document.querySelectorAll('#m-nav button')].filter(n=>n.getClientRects().length).map(n=>{const r=n.getBoundingClientRect();return{y:r.y,bottom:r.bottom,w:r.width,h:r.height}})})`);
    shot('360-gesture-ime');assert.equal(ime.active,'search');assert(ime.viewport.h<before.viewport.h-100,JSON.stringify({before,ime}));assert(ime.scrollWidth<=ime.viewport.w+1);
    adb('shell','input','keyevent','KEYCODE_BACK');await pause(300);const afterImeBack=await p.js(`({h:visualViewport.height,active:document.activeElement?.id,page:document.body.dataset.mpage})`);
    results.push({name:'360-gesture-ime-back',before,ime,afterImeBack});
  }finally{p.ws.close()}
  writeFileSync(`${out}/phone-grid.json`,JSON.stringify({serial,avd:'RustRssT5',android:say('shell','getprop','ro.build.version.release'),apkSha256,original,results,passed:true},null,2)+'\n');
  console.log(JSON.stringify({scenarios:results.map(x=>x.name),apkSha256}));
}finally{
  adb('shell','wm','size','reset');adb('shell','wm','density','reset');
  adb('shell','settings','put','system','user_rotation',original.rotation);
  adb('shell','settings','put','system','accelerometer_rotation',original.auto);
  adb('shell','settings','put','system','font_scale',original.font);
  setNav(original.nav==='0'?'threebutton':'gestural');
}
