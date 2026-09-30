// Installed-APK Android zh/en × light/dark reader comparison on the owned AVD.
// Reset the synthetic T4 fixture first, then run with ANDROID_SERIAL=emulator-5582 CDP_URL=...
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
const serial=process.env.ANDROID_SERIAL;assert.equal(serial,'emulator-5582');
const out=resolve(process.argv[2]);mkdirSync(out,{recursive:true});
const adb=(...a)=>execFileSync('/usr/lib/android-sdk/platform-tools/adb',['-s',serial,...a],{timeout:30000});
const target=(await(await fetch(process.env.CDP_URL||'http://127.0.0.1:9229/json')).json()).find(x=>x.type==='page'&&JSON.parse(x.description||'{}').attached);assert(target);
const ws=new WebSocket(target.webSocketDebuggerUrl);await new Promise((r,e)=>{ws.onopen=r;ws.onerror=e});
let id=0;const pending=new Map();ws.onmessage=e=>{const m=JSON.parse(e.data),v=pending.get(m.id);if(!v)return;pending.delete(m.id);m.error?v.reject(m.error):v.resolve(m.result)};
const call=(method,params={})=>new Promise((resolve,reject)=>{const key=++id;pending.set(key,{resolve,reject});ws.send(JSON.stringify({id:key,method,params}))});
async function js(expression){const r=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});assert(!r.exceptionDetails,JSON.stringify(r.exceptionDetails));return r.result.value;}
const wait=ms=>new Promise(r=>setTimeout(r,ms));
async function until(expr){for(let i=0;i<100;i++){try{if(await js(expr))return}catch{}await wait(100)}throw Error(`Timeout: ${expr}`)}
async function native(selector){const p=await js(`(() => {const n=document.querySelector(${JSON.stringify(selector)});if(!n)return null;n.scrollIntoView({block:'nearest'});const r=n.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2,dpr:devicePixelRatio,visible:r.width&&r.height&&r.y>=0&&r.y<visualViewport.height}})()`);assert(p?.visible,selector);adb('shell','input','tap',String(Math.round(p.x*p.dpr)),String(Math.round(JSON.parse(target.description).screenY+p.y*p.dpr)));await wait(260)}
const scenes=[];
try{
 for(const locale of ['zh-CN','en'])for(const theme of ['light','dark']){
   await js(`window.__TAURI__.core.invoke('set_ui_locale',{locale:${JSON.stringify(locale)}})`);
   await js(`window.__TAURI__.core.invoke('set_ui_theme',{theme:${JSON.stringify(theme)}})`);
   await call('Page.reload',{ignoreCache:true});
   await until(`!!document.querySelector('#m-seg-articles [data-mview="all"]')`);
   await native('#m-seg-articles [data-mview="all"]');
   await until(`!!document.querySelector('#entries li[data-id="1"]')`);
   await native('#entries li[data-id="1"] .title');
   await until(`document.body.dataset.mpage==='reader'`);
   await until(`!!document.querySelector('.reader-image-fallback')`);
   const data=await js(`(() => {const b=document.querySelector('.reader-head h1'),p=document.querySelector('.article p'),m=document.querySelector('.reader-head .meta'),r=document.querySelector('#reader'),v=visualViewport,s=getComputedStyle(document.body);return {viewport:{width:v.width,height:v.height},titleLines:b.getBoundingClientRect().height/parseFloat(getComputedStyle(b).lineHeight),firstY:p.getBoundingClientRect().top,firstRatio:p.getBoundingClientRect().top/v.height,metaHeight:m.getBoundingClientRect().height,buttons:[...document.querySelectorAll('.reader-actions button')].map(n=>({id:n.id,width:n.getBoundingClientRect().width,height:n.getBoundingClientRect().height,label:n.getAttribute('aria-label')||n.textContent})),back:{width:document.querySelector('#m-reader-back').getBoundingClientRect().width,height:document.querySelector('#m-reader-back').getBoundingClientRect().height},readerOverflow:r.scrollWidth>r.clientWidth,pageOverflow:document.documentElement.scrollWidth>v.width,background:s.backgroundColor,color:s.color,more:document.querySelector('#act-more').textContent,offlineFallback:document.querySelector('.reader-image-fallback')?.textContent}})()`);
   const settings=await js(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>({locale:s.locale,theme:s.theme}))`);
   assert.equal(settings.locale,locale);assert.equal(settings.theme,theme);
   assert.equal(data.viewport.width,360);assert(data.firstRatio<=.4);assert(data.metaHeight<=31);assert(!data.readerOverflow&&!data.pageOverflow);assert(data.buttons.every(b=>b.width>=48&&b.height>=48));assert(data.back.width>=48&&data.back.height>=48);
   const slug=(locale==='zh-CN'?'zh':'en')+'-'+theme;
   writeFileSync(`${out}/${slug}-reader.png`,adb('exec-out','screencap','-p'));
   scenes.push({slug,settings,...data});
   await native('#m-reader-back');await until(`document.body.dataset.mpage==='articles'`);
 }
 writeFileSync(`${out}/matrix-results.json`,JSON.stringify({serial,scenes},null,2)+'\n');
 console.log(JSON.stringify({scenes:scenes.map(s=>({slug:s.slug,firstRatio:s.firstRatio,titleLines:s.titleLines}))}));
}finally{ws.close()}
