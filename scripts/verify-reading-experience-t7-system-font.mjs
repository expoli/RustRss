// Real Android system font scaling on the owned 360 CSS px AVD display.
// ANDROID_SERIAL=emulator-5584 node scripts/verify-reading-experience-t7-system-font.mjs OUT
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import {mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import {resolve} from 'node:path';

const serial=process.env.ANDROID_SERIAL;assert.equal(serial,'emulator-5584');
const out=resolve(process.argv[2]);mkdirSync(out,{recursive:true});
const adb=(...args)=>execFileSync('/usr/lib/android-sdk/platform-tools/adb',['-s',serial,...args],{timeout:30000,maxBuffer:20*1024*1024});
const say=(...args)=>adb(...args).toString().trim(),wait=ms=>new Promise(ok=>setTimeout(ok,ms));
assert.match(say('emu','avd','name'),/RustRssT5/);
const apkSha256=createHash('sha256').update(readFileSync('src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk')).digest('hex');
const original={font:say('shell','settings','get','system','font_scale'),size:say('shell','wm','size')};
const checks=[];const shot=name=>writeFileSync(`${out}/${name}.png`,adb('exec-out','screencap','-p'));
async function attach(){
  adb('shell','am','force-stop','tech.expoli.rustrss');adb('shell','am','start','-n','tech.expoli.rustrss/.MainActivity');
  let pid='';for(let i=0;i<50;i++){try{pid=say('shell','pidof','tech.expoli.rustrss').split(' ')[0]}catch{}if(pid)break;await wait(200)}assert(pid);
  adb('forward','tcp:9237',`localabstract:webview_devtools_remote_${pid}`);
  let target;for(let i=0;i<50;i++){try{target=(await(await fetch('http://127.0.0.1:9237/json')).json()).find(t=>t.type==='page'&&JSON.parse(t.description||'{}').attached);if(target)break}catch{}await wait(200)}assert(target);
  const ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((ok,fail)=>{ws.onopen=ok;ws.onerror=fail});
  let id=0;const pending=new Map();ws.onmessage=e=>{const m=JSON.parse(e.data),p=pending.get(m.id);if(!p)return;clearTimeout(p.timer);pending.delete(m.id);m.error?p.fail(m.error):p.ok(m.result)};
  const js=expression=>new Promise((ok,fail)=>{const next=++id,timer=setTimeout(()=>{pending.delete(next);fail(Error('CDP timeout'))},15000);pending.set(next,{ok:r=>r.exceptionDetails?fail(Error(JSON.stringify(r.exceptionDetails))):ok(r.result.value),fail,timer});ws.send(JSON.stringify({id:next,method:'Runtime.evaluate',params:{expression,returnByValue:true,awaitPromise:true}}))});
  const until=async expr=>{for(let i=0;i<100;i++){try{if(await js(expr))return}catch{}await wait(100)}throw Error(`Timed out ${expr}`)};
  const tap=async selector=>{const r=await js(`(()=>{const n=document.querySelector(${JSON.stringify(selector)});n?.scrollIntoView({block:'center'});if(!n)return null;const r=n.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;return{x,y,w:r.width,h:r.height,dpr:devicePixelRatio,hit:document.elementFromPoint(x,y)?.closest(${JSON.stringify(selector)})===n}})()`);assert(r?.w>0&&r.h>0&&r.hit,JSON.stringify({selector,r}));adb('shell','input','tap',String(Math.round(r.x*r.dpr)),String(Math.round(JSON.parse(target.description).screenY+r.y*r.dpr)));await wait(300);return r};
  return {ws,js,until,tap,pid};
}
const geometry=()=>`(()=>{const box=s=>{const n=document.querySelector(s);if(!n)return null;const r=n.getBoundingClientRect();return{x:r.x,y:r.y,w:r.width,h:r.height,right:r.right,bottom:r.bottom,visible:!!n.getClientRects().length,font:getComputedStyle(n).fontSize}};return{viewport:{w:visualViewport.width,h:visualViewport.height},dpr:devicePixelRatio,scrollWidth:document.documentElement.scrollWidth,bodyFont:getComputedStyle(document.body).fontSize,title:box('#entries li[data-id] .title'),paragraph:box('.article p'),back:box('#m-reader-back'),nav:[...document.querySelectorAll('#m-nav button')].filter(n=>n.getClientRects().length).map(n=>{const r=n.getBoundingClientRect();return{w:r.width,h:r.height,x:r.x,right:r.right,bottom:r.bottom}})}})()`;
try{
  adb('shell','wm','size','945x2100');
  for(const scale of ['1.0','1.3','2.0']){
    adb('shell','settings','put','system','font_scale',scale);await wait(400);
    const p=await attach();
    try{
      await p.until(`!!document.querySelector('#entries li[data-id] .title')`);
      const list=await p.js(geometry());shot(`system-font-${scale}-list`);
      assert(list.scrollWidth<=list.viewport.w+1&&list.nav.length===4);
      assert(list.nav.every(n=>n.w>=48&&n.h>=48&&n.right<=list.viewport.w+1&&n.bottom<=list.viewport.h+1));
      await p.tap('#entries li[data-id] .title');await p.until(`document.body.dataset.mpage==='reader'`);
      const reader=await p.js(geometry());shot(`system-font-${scale}-reader`);
      assert(reader.scrollWidth<=reader.viewport.w+1&&reader.back.w>=48&&reader.back.h>=48);
      checks.push({scale,system:say('shell','settings','get','system','font_scale'),configuration:say('shell','dumpsys','window').match(/mGlobalConfiguration=\{[^}]*\}/)?.[0]||null,pid:p.pid,list,reader});
    }finally{p.ws.close()}
  }
  const fonts=checks.map(c=>Number.parseFloat(c.reader.paragraph.font));
  const webviewSystemFontScaling=fonts[2]/fonts[0];
  writeFileSync(`${out}/system-font-results.json`,JSON.stringify({serial,avd:'RustRssT5',apkSha256,original,checks,webviewSystemFontScaling,systemFontApplied:webviewSystemFontScaling>=1.8,passed:true},null,2)+'\n');
  console.log(JSON.stringify({scales:checks.map(x=>x.scale),readerFonts:fonts,webviewSystemFontScaling,apkSha256}));
}finally{adb('shell','settings','put','system','font_scale',original.font);adb('shell','wm','size','reset')}
