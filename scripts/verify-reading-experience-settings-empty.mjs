// Native empty-subscriptions/search actions on the task-owned RustRssT5 AVD.
// Run only after pm clear on emulator-5584; the app database must be empty.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

assert.equal(process.env.ANDROID_SERIAL, 'emulator-5584');
const out = resolve(process.argv[2]); mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', process.env.ANDROID_SERIAL, ...args], { timeout: 30000 });
const wait = ms => new Promise(done => setTimeout(done, ms));
const target = (await (await fetch(process.env.CDP_URL)).json()).find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target, 'Attached task-owned WebView missing');
const screenY = JSON.parse(target.description).screenY;
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((done, fail) => { socket.onopen = done; socket.onerror = fail; });
let seq = 0; const pending = new Map();
socket.onmessage = event => {
  const m = JSON.parse(event.data), slot = pending.get(m.id); if (!slot) return;
  clearTimeout(slot.timer); pending.delete(m.id);
  m.error ? slot.fail(m.error) : slot.done(m.result);
};
const js = expression => new Promise((done, fail) => {
  const id = ++seq, timer = setTimeout(() => { pending.delete(id); fail(Error('CDP timeout: ' + expression.slice(0, 80))); }, 15000);
  pending.set(id, { timer, done: r => r.exceptionDetails ? fail(Error(JSON.stringify(r.exceptionDetails))) : done(r.result.value), fail });
  socket.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, returnByValue: true, awaitPromise: true } }));
});
async function until(expression) {
  for (let i = 0; i < 100; i++) { if (await js(expression)) return; await wait(120); }
  throw Error('Timed out: ' + expression);
}
async function nativeTap(selector) {
  const point = await js(`(()=>{const n=document.querySelector(${JSON.stringify(selector)});n?.scrollIntoView({block:'nearest'});if(!n)return null;const r=n.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;return {x,y,dpr:devicePixelRatio,visible:r.width>=40&&r.height>=40&&y<visualViewport.height&&document.elementFromPoint(x,y)?.closest(${JSON.stringify(selector)})===n}})()`);
  assert(point?.visible, `Native target unavailable: ${selector} ${JSON.stringify(point)}`);
  adb('shell', 'input', 'tap', String(Math.round(point.x * point.dpr)), String(Math.round(screenY + point.y * point.dpr)));
  await wait(250);
}
const shot = name => writeFileSync(`${out}/${name}.png`, adb('exec-out', 'screencap', '-p'));
const checks = [];
const check = (name, ok, detail) => { assert(ok, `${name}: ${JSON.stringify(detail)}`); checks.push({ name, detail }); };
const geometry = () => js(`(()=>{const r=document.querySelector('#entries').getBoundingClientRect();return {list:[r.x,r.y,r.width,r.height],viewport:[visualViewport.width,visualViewport.height],pageWidth:document.documentElement.scrollWidth}})()`);
try {
  await until(`document.body.dataset.mpage==='articles'`);
  const feeds = await js(`window.__TAURI__.core.invoke('list_feeds')`);
  check('disposable-empty-database', feeds.length === 0, { count: feeds.length });
  await nativeTap('#m-nav [data-mpage-btn="subscriptions"]');
  await until(`document.body.dataset.mpage==='subscriptions' && !!document.querySelector('#m-feeds-empty')?.getClientRects().length`);
  const empty = await js(`(()=>{const n=document.querySelector('#m-feeds-empty'),b=document.querySelector('#m-empty-add-feed'),input=document.querySelector('#add-url');const tabbables=[...document.querySelectorAll('button,input')].filter(x=>x.getClientRects().length&&!x.disabled&&x.tabIndex>=0).map(x=>x.id||x.dataset.mpageBtn&&'nav-'+x.dataset.mpageBtn||x.className);return {copy:n.querySelector('p').textContent,button:b.textContent,buttonHeight:b.getBoundingClientRect().height,inputVisible:!!input.getClientRects().length,tabbables,role:b.getAttribute('role')||'button',pageWidth:document.documentElement.scrollWidth,viewportWidth:visualViewport.width}})()`);
  check('empty-subscriptions-copy-action', empty.copy.length > 15 && /Add feed|添加订阅/.test(empty.button) && empty.buttonHeight >= 48 && empty.inputVisible && empty.pageWidth <= empty.viewportWidth + 1, empty);
  check('empty-subscriptions-focus-order', empty.tabbables.indexOf('add-url') >= 0 && empty.tabbables.indexOf('add-url') < empty.tabbables.indexOf('m-empty-add-feed') && empty.tabbables.indexOf('m-empty-add-feed') < empty.tabbables.indexOf('nav-settings'), empty.tabbables);
  adb('shell', 'uiautomator', 'dump', '/sdcard/t5-empty-subscriptions.xml');
  const subscriptionsTree = adb('shell', 'cat', '/sdcard/t5-empty-subscriptions.xml').toString();
  check('empty-subscriptions-android-accessibility', subscriptionsTree.includes(`text="${empty.button}"`) &&
    (subscriptionsTree.includes('text="No subscriptions yet.') || subscriptionsTree.includes('text="还没有订阅。')));
  shot('android-empty-subscriptions');
  await nativeTap('#m-empty-add-feed');
  check('empty-subscriptions-add-focuses-existing-form', await js(`document.activeElement?.id==='add-url' && !document.querySelector('#add-row').classList.contains('hidden')`));
  adb('shell', 'input', 'keyevent', 'KEYCODE_BACK'); // close IME before navigation
  await until(`visualViewport.height>700`);
  await nativeTap('#m-nav [data-mpage-btn="articles"]');
  if (await js(`document.body.dataset.mpage!=='articles'`)) await nativeTap('#m-nav [data-mpage-btn="articles"]');
  await until(`document.body.dataset.mpage==='articles'`);
  await nativeTap('#btn-search-toggle');
  await js(`(()=>{const n=document.querySelector('#search');n.value='t5-no-results-impossible-token';n.dispatchEvent(new Event('input',{bubbles:true}));return true})()`);
  await until(`document.body.dataset.listKind==='search' && !!document.querySelector('#entries .empty-search-actions')`);
  adb('shell', 'input', 'keyevent', 'KEYCODE_BACK'); // search IME may be shown
  await wait(150);
  const noResults = await js(`(()=>{const li=document.querySelector('#entries li'),c=li.querySelector('.search-clear'),r=li.querySelector('.search-retry');const ids=[...document.querySelectorAll('button,input')].filter(x=>x.getClientRects().length&&!x.disabled&&x.tabIndex>=0).map(x=>x.id||x.className);return {message:li.querySelector('span')?.textContent,clear:c?.textContent,retry:r?.textContent,buttons:[c?.getBoundingClientRect().height,r?.getBoundingClientRect().height],ids,query:document.querySelector('#search').value,scope:document.querySelector('#search-scope').textContent,pageWidth:document.documentElement.scrollWidth,viewportWidth:visualViewport.width}})()`);
  check('empty-search-clear-retry-wording', noResults.message?.length > 10 && /Clear search|清除搜索/.test(noResults.clear) && /Retry search|重试搜索/.test(noResults.retry) && noResults.buttons.every(v => v >= 48) && noResults.scope.length > 0 && noResults.pageWidth <= noResults.viewportWidth + 1, noResults);
  check('empty-search-focus-order', noResults.ids.indexOf('search') < noResults.ids.indexOf('btn-search-cancel') && noResults.ids.indexOf('btn-search-cancel') < noResults.ids.indexOf('search-clear') && noResults.ids.indexOf('search-clear') < noResults.ids.indexOf('search-retry'), noResults.ids);
  adb('shell', 'uiautomator', 'dump', '/sdcard/t5-empty-search.xml');
  const searchTree = adb('shell', 'cat', '/sdcard/t5-empty-search.xml').toString();
  check('empty-search-android-accessibility', searchTree.includes(`text="${noResults.clear}"`) && searchTree.includes(`text="${noResults.retry}"`) && searchTree.includes(`text="${noResults.message}"`));
  await js(`document.querySelector('#entries .search-clear').focus();true`);
  adb('shell', 'input', 'keyevent', 'KEYCODE_TAB');
  check('empty-search-native-tab-clear-to-retry', await js(`document.activeElement?.classList.contains('search-retry')`));
  shot('android-empty-search');
  const beforeRetry = await geometry();
  await nativeTap('#entries .search-retry');
  await until(`document.body.dataset.listKind==='search' && !!document.querySelector('#entries .search-retry')`);
  const afterRetry = await geometry();
  const queryAfter = await js(`document.querySelector('#search').value`);
  check('empty-search-retry-retains-query-and-layout', queryAfter === noResults.query && JSON.stringify(beforeRetry) === JSON.stringify(afterRetry), { beforeRetry, afterRetry, queryAfter });
  await nativeTap('#entries .search-clear');
  await until(`document.body.dataset.listKind!=='search' && document.body.dataset.searchOpen==='false'`);
  check('empty-search-clear-restores-source', await js(`document.querySelector('#search').value==='' && document.body.dataset.mpage==='articles'`));
  const apk = readFileSync('src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk');
  const result = { device: process.env.ANDROID_SERIAL, apkSha256: createHash('sha256').update(apk).digest('hex'), checks };
  writeFileSync(`${out}/android-empty-results.json`, JSON.stringify(result, null, 2) + '\n');
  process.stdout.write(JSON.stringify({ checks: checks.length, apkSha256: result.apkSha256 }) + '\n');
} finally { socket.close(); }
