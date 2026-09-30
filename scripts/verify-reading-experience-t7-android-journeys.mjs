// Final APK: three native Android journeys on one disposable 30-row fixture.
// ANDROID_SERIAL=emulator-5584 node scripts/verify-reading-experience-t7-android-journeys.mjs OUT
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const serial=process.env.ANDROID_SERIAL;
assert.equal(serial,'emulator-5584','Only the task-owned RustRssT5 AVD is allowed');
const out=resolve(process.argv[2]);mkdirSync(out,{recursive:true});
const adb=(...args)=>execFileSync('/usr/lib/android-sdk/platform-tools/adb',['-s',serial,...args],{timeout:30000,maxBuffer:20*1024*1024});
const say=(...args)=>adb(...args).toString().trim();
assert.match(say('emu','avd','name'),/RustRssT5/);
const apk='src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk';
const apkSha256=createHash('sha256').update(readFileSync(apk)).digest('hex');
const fixtureSha256=createHash('sha256').update(readFileSync('/tmp/rustrss-t7-fixture.sqlite')).digest('hex');
const wait=ms=>new Promise(ok=>setTimeout(ok,ms));
const checks=[];const record=(name,detail)=>checks.push({name,detail});
let ws;
try {
  adb('shell','am','force-stop','tech.expoli.rustrss');
  adb('shell','run-as','tech.expoli.rustrss','cp','/data/local/tmp/rustrss-t7-fixture.sqlite','rustrss/rustrss.sqlite');
  adb('shell','run-as','tech.expoli.rustrss','rm','-f','rustrss/rustrss.sqlite-wal','rustrss/rustrss.sqlite-shm');
  adb('shell','am','start','-n','tech.expoli.rustrss/.MainActivity');
  let pid='';for(let i=0;i<60;i++){try{pid=say('shell','pidof','tech.expoli.rustrss').split(' ')[0]}catch{}if(pid)break;await wait(200)}
  assert(pid,'App process missing');
  adb('forward','tcp:9237',`localabstract:webview_devtools_remote_${pid}`);
  let target;for(let i=0;i<50;i++){try{target=(await (await fetch('http://127.0.0.1:9237/json')).json()).find(t=>t.type==='page'&&JSON.parse(t.description||'{}').attached);if(target)break}catch{}await wait(200)}
  assert(target,'Attached Android WebView missing');
  const screenY=JSON.parse(target.description).screenY;
  ws=new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((ok,fail)=>{ws.onopen=ok;ws.onerror=fail});
  let id=0;const pending=new Map();
  ws.onmessage=e=>{const message=JSON.parse(e.data),slot=pending.get(message.id);if(!slot)return;clearTimeout(slot.timer);pending.delete(message.id);message.error?slot.fail(message.error):slot.ok(message.result)};
  const call=(method,params={})=>new Promise((ok,fail)=>{const next=++id,timer=setTimeout(()=>{pending.delete(next);fail(Error(`CDP ${method}`))},15000);pending.set(next,{ok,fail,timer});ws.send(JSON.stringify({id:next,method,params}))});
  const js=async expression=>{const result=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});assert(!result.exceptionDetails,JSON.stringify(result.exceptionDetails));return result.result.value};
  const until=async expression=>{for(let i=0;i<100;i++){try{if(await js(expression))return}catch{}await wait(100)}throw Error(`Timed out: ${expression}`)};
  const shot=name=>writeFileSync(`${out}/${name}.png`,adb('exec-out','screencap','-p'));
  const snap=()=>js(`({page:document.body.dataset.mpage,view:document.body.dataset.listKind,searchOpen:document.body.dataset.searchOpen,css:[visualViewport.width,visualViewport.height],dpr:devicePixelRatio,scrollWidth:document.documentElement.scrollWidth,active:document.activeElement?.id})`);
  async function tap(selector){
    await js(`document.querySelector(${JSON.stringify(selector)})?.scrollIntoView({block:'center'})`);
    const p=await js(`(()=>{const n=document.querySelector(${JSON.stringify(selector)});if(!n)return null;const r=n.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;return {x,y,w:r.width,h:r.height,dpr:devicePixelRatio,hit:document.elementFromPoint(x,y)?.closest(${JSON.stringify(selector)})===n,viewport:visualViewport.height}})()`);
    assert(p&&p.w>0&&p.h>0&&p.hit&&p.y<p.viewport,`Native target ${selector}: ${JSON.stringify(p)}`);
    adb('shell','input','tap',String(Math.round(p.x*p.dpr)),String(Math.round(screenY+p.y*p.dpr)));
    await wait(250);return p;
  }
  async function menu(label){
    const selector=await js(`(()=>{const b=[...document.querySelectorAll('.ctx-sheet-list button')].find(x=>x.textContent.includes(${JSON.stringify(label)}));if(!b)return null;b.dataset.t7Target='1';return '.ctx-sheet-list button[data-t7-target="1"]'})()`);
    assert(selector,`Missing menu action ${label}`);
    const point=await tap(selector);await js(`document.querySelectorAll('[data-t7-target]').forEach(n=>delete n.dataset.t7Target)`);return point;
  }
  const core=(cmd,args={})=>js(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
  const back=()=>adb('shell','input','keyevent','KEYCODE_BACK');

  await until(`document.querySelectorAll('#entries li[data-id]').length===15`);
  assert.equal((await snap()).page,'articles');
  assert.equal((await core('list_feeds')).length,3);
  const initial=await snap();shot('01-articles-before');

  // Journey 1: choose a filter, read, save, return, then open Saved.
  await tap('[data-mview="all"]');
  await until(`document.querySelector('#views li.active')?.dataset.kind==='all'`);
  await tap('#entries li[data-id="1"] .title');
  await until(`document.body.dataset.mpage==='reader'`);
  const reader=await snap();shot('02-reader-before-save');
  assert(reader.scrollWidth<=reader.css[0]+1);
  await tap('#act-star');await until(`window.__TAURI__.core.invoke('get_entry',{id:1}).then(e=>e.starred===true)`);
  await tap('#act-later');await until(`window.__TAURI__.core.invoke('get_entry',{id:1}).then(e=>e.read_later===true)`);
  const saved=await core('get_entry',{id:1});shot('03-reader-saved');
  back();await until(`document.body.dataset.mpage==='articles'`);
  const returned=await snap();
  await tap('[data-mpage-btn="saved"]');
  await until(`document.body.dataset.mpage==='saved' && !!document.querySelector('#entries li[data-id="1"]')`);
  shot('04-saved-filter');
  record('filter-read-star-later-back-saved',{initial,reader,returned,entryId:1,read:saved.read,starred:saved.starred,readLater:saved.read_later,savedListHasArticle:await js(`!!document.querySelector('#entries li[data-id="1"]')`)});

  // Journey 2: source row More → Move to → named folder, with object ID readback.
  await tap('[data-mpage-btn="subscriptions"]');
  await until(`document.body.dataset.mpage==='subscriptions'`);
  const feedBefore=(await core('list_feeds')).find(f=>f.id===2);
  assert.equal(feedBefore.folder_id,null);
  await tap('#feeds li[data-feed-id="2"] .row-more');
  await until(`!!document.querySelector('.ctx-sheet-overlay')`);
  const menuTitle=await js(`document.querySelector('#ctx-sheet-title').textContent`);
  assert(menuTitle.includes(feedBefore.title));shot('05-feed-more');
  await menu('移动到');await until(`document.querySelectorAll('.ctx-sheet-list button').length>20`);
  shot('06-folder-choices');
  await menu('Folder 00');
  await until(`window.__TAURI__.core.invoke('list_feeds').then(rows=>rows.find(f=>f.id===2)?.folder_id===1)`);
  const feedAfter=(await core('list_feeds')).find(f=>f.id===2);
  shot('07-feed-moved');
  record('subscription-more-move-folder',{feedId:2,folderBefore:feedBefore.folder_id,folderAfter:feedAfter.folder_id,stableId:feedBefore.id===feedAfter.id,menuTitle});

  // Journey 3: settings font draft → preview (zero write) → save → edit/cancel.
  await tap('[data-mpage-btn="settings"]');
  await until(`document.body.dataset.mpage==='settings'`);
  await tap('#tab-appearance');
  await until(`document.querySelector('.settings-dialog').dataset.screen==='detail'`);
  await tap('#appearance-editor .theme-advanced summary');
  await until(`document.querySelector('#appearance-editor .theme-advanced').open`);
  const first=(await core('get_ui_settings')).theme_snapshot.config;
  const field='#appearance-editor [data-theme-field="typography.ui_size"]';
  const current=Number(await js(`document.querySelector(${JSON.stringify(field)}).value`));
  const next=current===17?18:17;
  async function typeSize(value){
    await tap(field);adb('shell','input','keyevent','KEYCODE_MOVE_END');
    for(let i=0;i<4;i++)adb('shell','input','keyevent','KEYCODE_DEL');
    adb('shell','input','text',String(value));
    await until(`document.querySelector(${JSON.stringify(field)}).value===${JSON.stringify(String(value))}`);
    adb('shell','input','keyevent','KEYCODE_TAB');await wait(200);
  }
  await typeSize(next);shot('08-font-draft');
  const draft=(await core('get_ui_settings')).theme_snapshot.config;
  assert.equal(draft.revision,first.revision);
  await tap('#appearance-editor .theme-editor-actions button:nth-child(2)');
  await wait(300);
  const preview=(await core('get_ui_settings')).theme_snapshot.config;
  assert.equal(preview.revision,first.revision);shot('09-font-preview');
  await tap('#appearance-editor .theme-editor-actions button:first-child');
  await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.theme_snapshot.config.revision===${first.revision+1})`);
  const persisted=(await core('get_ui_settings')).theme_snapshot.config;
  await typeSize(next+1);
  await tap('#appearance-editor .theme-editor-actions button:nth-child(3)');
  const cancelled=(await core('get_ui_settings')).theme_snapshot.config;
  assert.equal(cancelled.revision,persisted.revision);
  assert.equal(Number(await js(`document.querySelector(${JSON.stringify(field)}).value`)),next);
  shot('10-font-cancel');
  const savedSize=(await core('get_ui_settings')).theme_snapshot.light.typography.ui_size;
  assert.equal(savedSize,next);
  record('settings-font-draft-preview-save-cancel',{oldSize:current,newSize:next,revisionBefore:first.revision,draftRevision:draft.revision,previewRevision:preview.revision,savedRevision:persisted.revision,cancelRevision:cancelled.revision,savedSize});
  writeFileSync(`${out}/android-journeys.json`,JSON.stringify({serial,avd:'RustRssT5',pid,android:say('shell','getprop','ro.build.version.release'),apkSha256,fixtureSha256,physicalSize:say('shell','wm','size'),density:say('shell','wm','density'),screenY,checks,passed:true},null,2)+'\n');
  console.log(JSON.stringify({passed:checks.map(c=>c.name),apkSha256}));
} finally {ws?.close()}
