// Round-1 T4 reviewer follow-up on the rebuilt app and task-owned AVD.
// ANDROID_SERIAL=emulator-5582 node scripts/verify-reading-experience-review-android.mjs OUT
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const serial = process.env.ANDROID_SERIAL;
assert.equal(serial, 'emulator-5582');
const out = resolve(process.argv[2]); mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, ...args], { timeout: 30000, maxBuffer: 10 * 1024 * 1024 });
const wait = ms => new Promise(done => setTimeout(done, ms));
const target = (await (await fetch(process.env.CDP_URL || 'http://127.0.0.1:9229/json')).json())
  .find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target);
const page = JSON.parse(target.description);
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((done, fail) => { socket.onopen = done; socket.onerror = fail; });
let sequence = 0;
const pending = new Map();
socket.onmessage = event => {
  const reply = JSON.parse(event.data), slot = pending.get(reply.id);
  if (!slot) return;
  clearTimeout(slot.timer); pending.delete(reply.id);
  reply.error ? slot.fail(reply.error) : slot.done(reply.result);
};
const call = (method, params = {}) => new Promise((done, fail) => {
  const id = ++sequence, timer = setTimeout(() => { pending.delete(id); fail(new Error(`CDP ${method} timeout`)); }, 15000);
  pending.set(id, { done, fail, timer }); socket.send(JSON.stringify({ id, method, params }));
});
async function js(expression) {
  const answer = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  assert(!answer.exceptionDetails, JSON.stringify(answer.exceptionDetails));
  return answer.result.value;
}
async function until(expression) {
  for (let i = 0; i < 120; i++) { try { if (await js(expression)) return; } catch { /* transient navigation */ } await wait(100); }
  throw new Error(`Timed out: ${expression}`);
}
async function tap(selector) {
  const rect = await js(`(() => {const n=document.querySelector(${JSON.stringify(selector)});if(!n)return null;n.scrollIntoView({block:'nearest'});const r=n.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;return {x,y,width:r.width,height:r.height,dpr:devicePixelRatio,visible:r.width>0&&r.height>0&&y>=0&&y<visualViewport.height&&document.elementFromPoint(x,y)?.closest(${JSON.stringify(selector)})===n}})()`);
  assert(rect?.visible, `Not tappable ${selector}: ${JSON.stringify(rect)}`);
  adb('shell', 'input', 'tap', String(Math.round(rect.x * rect.dpr)), String(Math.round(page.screenY + rect.y * rect.dpr)));
  await wait(300);
}
function back() { adb('shell', 'input', 'keyevent', 'KEYCODE_BACK'); }
function shot(name) { writeFileSync(`${out}/${name}.png`, adb('exec-out', 'screencap', '-p')); }
const entry = id => js(`window.__TAURI__.core.invoke('get_entry',{id:${id}})`);
const checks = [];
try {
  await until(`!!document.querySelector('#entries li[data-id="1"] .title')`);
  await tap('#entries li[data-id="1"] .title');
  await until(`document.body.dataset.mpage==='reader'`);
  const storedBefore = (await entry(1)).read;
  assert.equal(storedBefore, true);
  assert(await js(`(() => {window.__T4_ERROR_EVENTS=[];window.__T4_NAV_EVENTS=[];for(const name of ['popstate','rustrss-menu-close','rustrss-menu-action'])window.addEventListener(name,()=>window.__T4_NAV_EVENTS.push(name));window.addEventListener('unhandledrejection',e=>window.__T4_ERROR_EVENTS.push(String(e.reason)));window.__T4_ORIGINAL_CORE=window.__TAURI__.core;window.__T4_REJECTS=0;window.__TAURI__.core={...window.__T4_ORIGINAL_CORE,invoke:(cmd,args)=>{if(cmd==='set_read'){window.__T4_REJECTS++;return Promise.reject(new Error('synthetic T4 write failure'))}return window.__T4_ORIGINAL_CORE.invoke(cmd,args)}};return window.__TAURI__.core.invoke!==window.__T4_ORIGINAL_CORE.invoke})()`));
  await tap('#act-more');
  await until(`!!document.querySelector('.ctx-sheet-list')`);
  assert(await js(`(() => {const n=[...document.querySelectorAll('.ctx-sheet-list button')].find(n=>n.textContent.includes('标为未读'));if(!n)return false;n.dataset.t4Reject='true';return true})()`));
  await tap('.ctx-sheet-list button[data-t4-reject="true"]');
  await until(`window.__T4_REJECTS===1 && document.querySelector('#m-status-text')?.classList.contains('error')`);
  const failed = await js(`(() => {const n=document.querySelector('#m-status'),r=n.getBoundingClientRect();return {attempts:window.__T4_REJECTS,status:n.textContent,error:document.querySelector('#m-status-text').classList.contains('error'),visible:getComputedStyle(n).display!=='none'&&r.height>0,statusBottom:r.bottom,viewportHeight:visualViewport.height,unhandled:window.__T4_ERROR_EVENTS,readClass:document.querySelector('#entries li[data-id="1"]').classList.contains('read'),reader:document.body.dataset.mpage}})()`);
  assert.equal((await entry(1)).read, true);
  assert(failed.visible && failed.status.includes('synthetic T4 write failure') && failed.statusBottom <= failed.viewportHeight + 1 && failed.readClass && failed.reader === 'reader' && failed.unhandled.length === 0, JSON.stringify(failed));
  assert(!await js(`!!document.querySelector('.ctx-sheet-list')`));
  shot('review-read-failure-visible');
  await js(`(() => {window.__T4_REAL_WRITES=0;window.__TAURI__.core={...window.__T4_ORIGINAL_CORE,invoke:(cmd,args)=>{if(cmd==='set_read')window.__T4_REAL_WRITES++;return window.__T4_ORIGINAL_CORE.invoke(cmd,args)}};return true})()`);
  await tap('#act-more');
  await until(`!!document.querySelector('.ctx-sheet-list')`);
  assert(await js(`(() => {const n=[...document.querySelectorAll('.ctx-sheet-list button')].find(n=>n.textContent.includes('标为未读'));if(!n)return false;n.dataset.t4Retry='true';return true})()`));
  await tap('.ctx-sheet-list button[data-t4-retry="true"]');
  await until(`window.__TAURI__.core.invoke('get_entry',{id:1}).then(e=>!e.read)`);
  assert.equal(await js(`window.__T4_REAL_WRITES`), 1, 'successful retry must send exactly one write');
  checks.push({ name: 'rejected-reader-mark-unread-visible-and-unchanged', before: storedBefore, failure: failed, storedAfterFailure: true, storedAfterRetry: (await entry(1)).read, retryWrites: 1, navigationEvents: await js(`window.__T4_NAV_EVENTS`) });

  const windowState = adb('shell', 'dumpsys', 'window').toString();
  const status = windowState.match(/type=statusBars frame=\[0,0\]\[(\d+),(\d+)\]/);
  const navigation = windowState.match(/type=navigationBars frame=\[0,(\d+)\]\[(\d+),(\d+)\]/);
  assert(status && navigation, 'Android system bar InsetsSource unavailable');
  const scale = await js(`devicePixelRatio`);
  const safe = { statusBottom: Number(status[2]), navigationTop: Number(navigation[1]), webviewTop: page.screenY, webviewBottom: page.screenY + page.height,
    viewport: await js(`({width:visualViewport.width,height:visualViewport.height,scale:devicePixelRatio})`) };
  assert.equal(safe.webviewTop, safe.statusBottom);
  assert.equal(safe.webviewBottom, safe.navigationTop);
  assert(Math.abs(safe.viewport.height * scale - page.height) < 3);
  checks.push({ name: 'android-system-bar-safe-area', ...safe });

  const sizeBefore = await js(`parseFloat(getComputedStyle(document.querySelector('.article')).fontSize)`);
  assert.equal(sizeBefore, 14, 'Reset the isolated fixture before this review pass');
  async function saveSize(size) {
    await tap('#act-aa'); await until(`document.querySelector('#aa-dialog')?.open`);
    await js(`(() => {const n=document.querySelector('#aa-editor [data-theme-field="typography.read_size"]');n.value=${JSON.stringify(String(size))};n.dispatchEvent(new Event('change',{bubbles:true}));return true})()`);
    await tap('#aa-editor .theme-editor-actions button:first-child');
    await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.theme_snapshot.config.overrides.typography.read_size===${size})`);
    await tap('#aa-close');
    await until(`!document.querySelector('#aa-dialog')?.open`);
    await until(`parseFloat(getComputedStyle(document.querySelector('.article')).fontSize)===${size}`);
  }
  async function geometry() {
    return js(`(() => {const rr=s=>{const n=document.querySelector(s),r=n.getBoundingClientRect();return {x:r.x,y:r.y,right:r.right,bottom:r.bottom,width:r.width,height:r.height}};const reader=document.querySelector('#reader'),article=document.querySelector('.article');return {size:parseFloat(getComputedStyle(article).fontSize),viewport:{width:visualViewport.width,height:visualViewport.height},title:rr('.reader-head h1'),back:rr('#m-reader-back'),actions:[...document.querySelectorAll('.reader-actions button')].map(n=>({id:n.id,...rr('#'+n.id)})),readerOverflow:reader.scrollWidth>reader.clientWidth,articleOverflow:article.scrollWidth>article.clientWidth,pageOverflow:document.documentElement.scrollWidth>visualViewport.width+1,readerScrollTop:reader.scrollTop,readerScrollHeight:reader.scrollHeight,readerClientHeight:reader.clientHeight,first:rr('.article p:first-of-type'),last:rr('.article > :last-child')}})()`);
  }
  async function inspectSize(size, label) {
    await saveSize(size);
    await js(`document.querySelector('#reader').scrollTop=0`);
    const top = await geometry();
    assert.equal(top.size, size);
    assert(!top.readerOverflow && !top.articleOverflow && !top.pageOverflow, JSON.stringify(top));
    assert(top.back.x >= 0 && top.back.right <= top.viewport.width + 1 && top.back.y >= 0 && top.back.bottom <= top.viewport.height + 1);
    assert(top.actions.every(a => a.width >= 48 && a.height >= 48 && a.x >= 0 && a.right <= top.viewport.width + 1 && a.y >= 0 && a.bottom <= top.viewport.height + 1));
    shot(`review-aa-${label}-top`);
    await js(`document.querySelector('#reader').scrollTop=document.querySelector('#reader').scrollHeight`);
    const bottom = await geometry();
    assert(bottom.readerScrollTop > 0 && bottom.last.bottom <= bottom.viewport.height + 1 && bottom.last.bottom > 0, JSON.stringify(bottom));
    assert(!bottom.readerOverflow && !bottom.articleOverflow && !bottom.pageOverflow);
    assert(bottom.actions.every(a => a.y >= 0 && a.bottom <= bottom.viewport.height + 1));
    await tap('#act-more'); await until(`!!document.querySelector('.ctx-sheet-list')`);
    back(); await until(`!document.querySelector('.ctx-sheet-list')`);
    assert.equal(await js(`document.activeElement?.id`), 'act-more');
    assert.equal(await js(`window.__T4_REAL_WRITES`), 1, 'Back cancellation must not write');
    shot(`review-aa-${label}-bottom`);
    return { top, bottom, effectiveScale: size / sizeBefore };
  }
  checks.push({ name: 'aa-effective-1.3-long-content-reachable', ...await inspectSize(18, '18') });
  checks.push({ name: 'aa-effective-2.0-long-content-reachable', ...await inspectSize(28, '28') });
  writeFileSync(`${out}/review-android-results.json`, JSON.stringify({ serial, apk: process.env.APK_SHA256 || null, checks }, null, 2) + '\n');
  console.log(JSON.stringify({ checks: checks.map(c => c.name) }));
} finally {
  try { await js(`if(window.__T4_ORIGINAL_CORE)window.__TAURI__.core=window.__T4_ORIGINAL_CORE`); } catch {}
  socket.close();
}
