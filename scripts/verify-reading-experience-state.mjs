// State/count/sort/bulk/paging/retry/refresh on an isolated 200-row Android fixture.
// ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9229/json node scripts/verify-reading-experience-state.mjs OUT
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const serial = process.env.ANDROID_SERIAL;
assert(serial);
const out = resolve(process.argv[2]); mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, ...args], { timeout: 30000 });
const pause = ms => new Promise(done => setTimeout(done, ms));
const target = (await (await fetch(process.env.CDP_URL || 'http://127.0.0.1:9229/json')).json())
  .find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target);
const screenY = JSON.parse(target.description).screenY;
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((done, reject) => { ws.onopen = done; ws.onerror = reject; });
let id = 0; const pending = new Map();
ws.onmessage = event => { const m=JSON.parse(event.data), p=pending.get(m.id); if(!p)return; clearTimeout(p.timer);pending.delete(m.id);m.error?p.reject(m.error):p.resolve(m.result); };
const call = (method, params={}) => new Promise((done,reject)=>{const next=++id,timer=setTimeout(()=>{pending.delete(next);reject(new Error(method));},15000);pending.set(next,{resolve:done,reject,timer});ws.send(JSON.stringify({id:next,method,params}));});
async function js(expression){const r=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});assert(!r.exceptionDetails,JSON.stringify(r.exceptionDetails));return r.result.value;}
async function until(expression, attempts=120){for(let i=0;i<attempts;i++){try{if(await js(expression))return;}catch{}await pause(100);}throw new Error(`Timed out: ${expression}`);}
async function native(selector){
  await js(`document.querySelector(${JSON.stringify(selector)})?.scrollIntoView({block:'center'})`);
  const p=await js(`(()=>{const n=document.querySelector(${JSON.stringify(selector)});if(!n)return null;const r=n.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;return{x,y,dpr:devicePixelRatio,visible:r.width>=40&&r.height>=40&&y<visualViewport.height&&document.elementFromPoint(x,y)?.closest(${JSON.stringify(selector)})===n}})()`);
  assert(p?.visible, `${selector}: ${JSON.stringify(p)}; ${JSON.stringify(await js(`(()=>{const n=document.querySelector(${JSON.stringify(selector)});const r=n?.getBoundingClientRect();return{rect:r&&{x:r.x,y:r.y,w:r.width,h:r.height},hit:r&&document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)?.outerHTML.slice(0,300),html:n?.outerHTML.slice(0,300)}})()`))}`);
  adb('shell','input','tap',String(Math.round(p.x*p.dpr)),String(Math.round(screenY+p.y*p.dpr)));
  await pause(300);
}
async function menu(label){
  const index=await js(`(()=>[...document.querySelectorAll('#ctx-menu button')].findIndex(n=>n.textContent.includes(${JSON.stringify(label)})))()`);
  assert(index>=0,`Menu item ${label}`);
  await native(`#ctx-menu button:nth-of-type(${index+1})`);
}
const head=()=>js(`[...document.querySelectorAll('#entries li[data-id]')].slice(0,5).map(n=>n.dataset.id)`);
const rows=()=>js(`document.querySelectorAll('#entries li[data-id]').length`);
const count=()=>js(`document.querySelector('#list-count').textContent`);
const shot=name=>writeFileSync(`${out}/${name}.png`,adb('exec-out','screencap','-p'));
const checks=[];const check=(name,value)=>checks.push({name,...value});
try{
  await js(`window.__TAURI__.core.invoke('set_ui_locale',{locale:'zh-CN'})`);
  await call('Page.reload',{ignoreCache:true});
  await until(`document.documentElement.lang==='zh-CN'`);
  await until(`document.querySelectorAll('#entries li[data-id]').length===100`);
  const viewport=await js(`({width:visualViewport.width,height:visualViewport.height})`);
  await native('[data-mview="all"]');
  await until(`document.querySelectorAll('#entries li[data-id]').length===200`);
  const initialHead=await head(), initialCount=await count(), listMs=await js(`window.__LIST_MS`);
  assert(initialCount.includes('200') && new Set(await js(`[...document.querySelectorAll('#entries li[data-id]')].map(n=>n.dataset.id)`)).size===200);
  const first=initialHead[0];
  await js(`window.__t3First=document.querySelector('#entries li[data-id="${first}"]')`);
  await native(`#entries li[data-id="${first}"]`);
  await until(`document.body.dataset.mpage==='reader'`);
  adb('shell','input','keyevent','KEYCODE_BACK');
  await until(`document.body.dataset.mpage==='articles'`);
  assert(await js(`window.__t3First===document.querySelector('#entries li[data-id="${first}"]')`));
  check('200-rows-count-and-open-node-identity',{viewport,count:initialCount,head:initialHead,listMs,rowPreserved:true});

  // A full first page has an active sentinel. Fault inject only the cursor
  // request, then retry against real SQLite without changing filter or rows.
  await js(`window.__t3Core=window.__TAURI__.core;window.__t3PageFail=true;
    window.__TAURI__.core={...window.__t3Core,invoke:(command,args)=>command==='list_entries'&&args?.cursorId&&window.__t3PageFail?Promise.reject(new Error('fixture page offline')):window.__t3Core.invoke(command,args)}`);
  // Scrolling the sentinel into view can trigger its IntersectionObserver
  // before a native tap. Keep the fault active and observe that automatic
  // continuation fails into the manual Retry state.
  await js(`document.querySelector('#entries .load-sentinel button').scrollIntoView({block:'center'})`);
  await until(`document.querySelector('#entries .load-sentinel button')?.textContent.includes('重试')`);
  const failed=await js(`document.querySelector('#entries .load-sentinel').textContent`);
  assert.equal(await rows(),200);
  await js(`window.__t3PageFail=false`);
  await native('#entries .load-sentinel button');
  await until(`!document.querySelector('#entries .load-sentinel button')`);
  assert.equal(await rows(),200);
  await js(`window.__TAURI__.core=window.__t3Core`);
  check('paging-failure-retry-terminal-no-duplicates',{failed,terminal:await js(`document.querySelector('#entries .load-sentinel').textContent`),rows:await rows(),count:await count()});

  await native('#btn-list-sort'); await menu('最早在前');
  await until(`document.querySelector('#entries li[data-id]')?.dataset.id!==${JSON.stringify(first)}`);
  const oldestHead=await head();
  await native('#btn-list-sort'); await menu('未读优先');
  await until(`document.querySelector('#entries li[data-id]')?.dataset.id!==${JSON.stringify(oldestHead[0])}`);
  const unreadHead=await head();
  await native('#btn-list-sort'); await menu('最新在前');
  await until(`document.querySelector('#entries li[data-id]')?.dataset.id===${JSON.stringify(first)}`);
  const settings=await js(`window.__TAURI__.core.invoke('get_ui_settings')`);
  check('three-sort-modes-and-persisted-setting',{initialHead,oldestHead,unreadHead,restoredHead:await head(),sort:settings.list_sort??settings.list?.sort??null});

  const unreadBeforeHide=await js(`window.__TAURI__.core.invoke('list_entries',{limit:200,unreadOnly:true}).then(rows=>rows.length)`);
  await native('#btn-list-sort'); await menu('隐藏已读');
  await until(`document.querySelectorAll('#entries li[data-id]').length===${unreadBeforeHide}`);
  const hidden=await count();
  await native('#btn-list-sort'); await menu('隐藏已读');
  await until(`document.querySelectorAll('#entries li[data-id]').length===200`);
  check('hide-read-and-restore',{hidden,restored:await count()});

  await native('#btn-list-bulk'); await menu('当前视图全部已读');
  await until(`window.__TAURI__.core.invoke('list_entries',{limit:1,unreadOnly:true}).then(rows=>rows.length===0)`);
  const afterRead=await count();
  await native('#btn-list-bulk'); await menu('当前视图全部未读');
  await until(`window.__TAURI__.core.invoke('list_entries',{limit:1,unreadOnly:true}).then(rows=>rows.length===1)`);
  const afterUnread=await count();
  check('bulk-read-unread-current-all-scope',{afterRead,afterUnread,allRows:await rows()});

  await js(`document.querySelector('#entries').scrollTop=0`);
  const before=await js(`(()=>{const r=n=>{const b=n.getBoundingClientRect();return{y:b.y,h:b.height}};return{status:r(document.querySelector('#m-status')),nav:r(document.querySelector('#m-nav')),top:document.querySelector('#entries li[data-id]')?.dataset.id}})()`);
  await native('#btn-refresh');
  await until(`!document.querySelector('#btn-refresh').disabled`,300);
  const after=await js(`(()=>{const r=n=>{const b=n.getBoundingClientRect();return{y:b.y,h:b.height}};return{status:r(document.querySelector('#m-status')),nav:r(document.querySelector('#m-nav')),top:document.querySelector('#entries li[data-id]')?.dataset.id,text:document.querySelector('#m-status-text').textContent}})()`);
  assert(Math.abs(before.nav.y-after.nav.y)<1 && Math.abs(before.status.y-after.status.y)<1);
  check('refresh-feedback-stable-footer',{before,after});
  shot('state-after-refresh');
  writeFileSync(`${out}/results.json`,JSON.stringify({serial,fixtureSha256:'f338b98364e015a907e5904f92944f5b3d24592915dc49136d20b3e88db4676b',checks},null,2)+'\n');
  console.log(JSON.stringify({passed:checks.length,checks:checks.map(c=>c.name)}));
}finally{ws.close();}
