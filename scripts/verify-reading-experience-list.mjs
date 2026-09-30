// Real Android WebView + native tap/Back verification on the task-owned AVD.
// ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9229/json node scripts/verify-reading-experience-list.mjs OUT
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const serial = process.env.ANDROID_SERIAL;
assert(serial);
const out = resolve(process.argv[2]);
mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, ...args], { timeout: 30000 });
const pause = ms => new Promise(done => setTimeout(done, ms));
const target = (await (await fetch(process.env.CDP_URL || 'http://127.0.0.1:9229/json')).json())
  .find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target, 'Attached WebView');
const screenY = JSON.parse(target.description).screenY;
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((done, reject) => { ws.onopen = done; ws.onerror = reject; });
let id = 0;
const pending = new Map();
ws.onmessage = event => {
  const message = JSON.parse(event.data), slot = pending.get(message.id);
  if (!slot) return;
  clearTimeout(slot.timer); pending.delete(message.id);
  message.error ? slot.reject(message.error) : slot.resolve(message.result);
};
const call = (method, params = {}) => new Promise((done, reject) => {
  const next = ++id, timer = setTimeout(() => { pending.delete(next); reject(new Error(method)); }, 15000);
  pending.set(next, { resolve: done, reject, timer });
  ws.send(JSON.stringify({ id: next, method, params }));
});
async function js(expression) {
  const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  assert(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
  return result.result.value;
}
async function until(expression) {
  for (let i = 0; i < 100; i++) {
    try { if (await js(expression)) return; } catch { /* a reload can replace execution context */ }
    await pause(100);
  }
  throw new Error(`Timed out: ${expression}`);
}
async function native(selector) {
  await js(`document.querySelector(${JSON.stringify(selector)})?.scrollIntoView({block:'center'})`);
  const point = await js(`(() => { const n=document.querySelector(${JSON.stringify(selector)}); if(!n) return null;
    const r=n.getBoundingClientRect(); const x=r.x+r.width/2,y=r.y+r.height/2;
    return {x,y,dpr:devicePixelRatio,visible:r.width>=40&&r.height>=40&&y<visualViewport.height&&document.elementFromPoint(x,y)?.closest(${JSON.stringify(selector)})===n}; })()`);
  assert(point?.visible, `Unreachable native target ${selector}: ${JSON.stringify(point)}`);
  adb('shell', 'input', 'tap', String(Math.round(point.x * point.dpr)), String(Math.round(screenY + point.y * point.dpr)));
  await pause(300);
}
const shot = name => writeFileSync(`${out}/${name}.png`, adb('exec-out', 'screencap', '-p'));
const checks = [];
const check = (name, value) => checks.push({ name, ...value });
const kind = () => js(`document.body.dataset.listKind`);
const ids = () => js(`[...document.querySelectorAll('#entries li[data-id]')].map(n=>n.dataset.id)`);
const count = () => js(`document.querySelector('#list-count').textContent`);
try {
  await until(`!!document.querySelector('#entries li[data-id]')`);
  const viewport = await js(`({width:visualViewport.width,height:visualViewport.height})`);
  const startIds = await ids();
  assert.equal(await kind(), 'unread');
  assert.equal(startIds.length, 15);
  await native('[data-mpage-btn="saved"]');
  await until(`document.body.dataset.mpage==='saved' && document.querySelector('#views li.active')?.dataset.kind==='starred'`);
  const starred = await ids();
  assert(starred.length > 0 && starred.length < 30);
  await native('[data-mview="later"]');
  await until(`document.querySelector('#views li.active')?.dataset.kind==='later'`);
  const later = await ids();
  assert(later.length > 0 && later.length < 30);
  await native('[data-mpage-btn="articles"]');
  await native('[data-mview="all"]');
  await until(`document.querySelector('#views li.active')?.dataset.kind==='all'`);
  assert.equal((await ids()).length, 30);
  check('four-navigation-and-primary-filters', { viewport, unread: startIds.length, all: (await ids()).length, starred: starred.length, later: later.length, savedActive: await kind() });

  // Scroll to a nonzero anchor; search is intentionally library wide, then
  // article open/Android Back and Cancel must preserve the prior all view.
  const anchor = await js(`(() => {const l=document.querySelector('#entries');l.scrollTop=500;
    const t=l.getBoundingClientRect().top,r=[...l.querySelectorAll('li[data-id]')].find(n=>n.getBoundingClientRect().bottom>t);
    return {id:r.dataset.id,offset:r.getBoundingClientRect().top-t,scroll:l.scrollTop};})()`);
  await native('#btn-search-toggle');
  await until(`document.body.dataset.searchOpen==='true' && document.activeElement?.id==='search'`);
  // Android WebView can focus a field from a button without raising the IME;
  // the direct input tap is the real keyboard interaction.
  await native('#search');
  await until(`visualViewport.height < ${viewport.height - 100}`);
  const imeViewport = await js(`({height:visualViewport.height,input:document.activeElement.id,scope:document.querySelector('#search-scope').textContent})`);
  assert(imeViewport.scope.length && imeViewport.height < viewport.height, 'Native IME should shrink WebView');
  adb('shell', 'input', 'text', 'English');
  await until(`document.body.dataset.listKind==='search' && document.querySelector('#entries li[data-id]')`);
  const search = { count: await count(), ids: await ids(), scope: imeViewport.scope };
  assert(search.ids.length > 0 && !search.count.includes('/'));
  shot('search-ime');
  adb('shell', 'input', 'keyevent', 'KEYCODE_BACK');
  await pause(300); // close IME before selecting a result
  const first = search.ids[0];
  const identityBefore = await js(`(() => {window.__t3Row=document.querySelector('#entries li[data-id="${first}"]');return window.__t3Row?.dataset.id})()`);
  await native(`#entries li[data-id="${first}"]`);
  await until(`document.body.dataset.mpage==='reader'`);
  adb('shell', 'input', 'keyevent', 'KEYCODE_BACK');
  await until(`document.body.dataset.mpage==='articles'`);
  assert(await js(`window.__t3Row===document.querySelector('#entries li[data-id="${first}"]')`));
  check('search-query-reader-system-back-identity', { identityBefore, after: await js(`document.body.dataset.mpage`), rowPreserved: true, search });
  await native('#btn-search-cancel');
  await until(`document.querySelector('#views li.active')?.dataset.kind==='all' && document.body.dataset.searchOpen==='false'`);
  const restored = await js(`(() => {const l=document.querySelector('#entries'),r=l.querySelector('li[data-id="${anchor.id}"]');
    return {id:r?.dataset.id,offset:r?.getBoundingClientRect().top-l.getBoundingClientRect().top,scroll:l.scrollTop};})()`);
  assert.equal(restored.id, anchor.id);
  assert(Math.abs(restored.offset-anchor.offset)<3, JSON.stringify({anchor,restored}));
  check('cancel-restores-filter-and-row-anchor', { anchor, restored, kind: await kind() });

  // No match is an explicit empty state and retains the Cancel target.
  await native('#btn-search-toggle');
  await native('#search');
  adb('shell', 'input', 'text', 'zzzznoresult999');
  await until(`document.body.dataset.listKind==='search' && !document.querySelector('#entries li[data-id]')`);
  const empty = await js(`document.querySelector('#entries').textContent`);
  assert(empty.length && !empty.includes('undefined'));
  await native('#btn-search-cancel');
  await until(`document.querySelector('#views li.active')?.dataset.kind==='all'`);
  check('search-empty-and-cancel', { empty, restoredKind: await kind() });

  // Search IPC failure is a local fault injection; retry executes the real IPC.
  await js(`window.__t3Core=window.__TAURI__.core;
    window.__TAURI__.core={...window.__t3Core,invoke:(command,args)=>command==='search'&&window.__t3FailSearch ? Promise.reject(new Error('fixture offline')) : window.__t3Core.invoke(command,args)};
    window.__t3FailSearch=true;`);
  await native('#btn-search-toggle');
  await native('#search');
  adb('shell', 'input', 'text', 'Reading');
  await until(`document.body.dataset.listKind==='search' && !!document.querySelector('#entries li.dim button')`);
  const error = await js(`document.querySelector('#entries').textContent`);
  await js(`window.__t3FailSearch=false`);
  await native('#entries li.dim button');
  await until(`!!document.querySelector('#entries li[data-id]')`);
  const retried = await ids();
  assert(retried.length > 0);
  await native('#btn-search-cancel');
  await until(`document.querySelector('#views li.active')?.dataset.kind==='all'`);
  await js(`window.__TAURI__.core=window.__t3Core`);
  check('search-error-and-retry', { error, retried: retried.length });
  shot('return-list');
  writeFileSync(`${out}/results.json`, JSON.stringify({ serial, viewport, checks }, null, 2) + '\n');
  console.log(JSON.stringify({ passed: checks.length, checks: checks.map(c=>c.name) }));
} finally { ws.close(); }
