// Run against an isolated, awake emulator with the rebuilt debug APK installed.
// adb forward tcp:9227 localabstract:webview_devtools_remote_<app-pid>
// ANDROID_SERIAL=emulator-5580 node scripts/verify-android-mobile.mjs <evidence-dir>
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const serial = process.env.ANDROID_SERIAL;
assert(serial, 'Set ANDROID_SERIAL to the isolated test emulator');
const adb = process.env.ADB || '/usr/lib/android-sdk/platform-tools/adb';
const out = resolve(process.argv[2] || '/tmp/rustrss-android-mobile');
mkdirSync(out, { recursive: true });
const run = (...args) => execFileSync(adb, ['-s', serial, ...args], { timeout: 20000 });
const wait = ms => new Promise(r => setTimeout(r, ms));
const tabs = await (await fetch(process.env.CDP_URL || 'http://127.0.0.1:9227/json')).json();
const ws = new WebSocket(tabs.find(t => t.type === 'page').webSocketDebuggerUrl);
await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
let seq = 0;
const pending = new Map();
ws.onmessage = event => {
  const message = JSON.parse(event.data);
  const p = pending.get(message.id);
  if (p) { pending.delete(message.id); message.error ? p.reject(message.error) : p.resolve(message.result); }
};
function call(method, params) {
  const id = ++seq;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject }); ws.send(JSON.stringify({ id, method, params }));
  });
}
async function evaluate(expression) {
  const r = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  assert(!r.exceptionDetails, JSON.stringify(r.exceptionDetails)); return r.result.value;
}
const click = async selector => { await evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`); await wait(350); };
const screenshot = name => writeFileSync(`${out}/${name}.png`, run('exec-out', 'screencap', '-p'));
async function nativeButton(text) {
  // WebView accessibility can become ready after the JS command bindings.
  let node;
  for (let i = 0; i < 5; i++) {
    run('shell', 'uiautomator', 'dump', '/sdcard/mobile-test.xml');
    const xml = run('shell', 'cat', '/sdcard/mobile-test.xml').toString();
    node = [...xml.matchAll(/<node\b[^>]*>/g)].map(m => m[0])
      .find(n => n.includes(`text="${text}"`) && n.includes('class="android.widget.Button"'));
    if (node) break;
    await wait(200);
  }
  assert(node, `Native button not found: ${text}`);
  const [, x1, y1, x2, y2] = node.match(/bounds="\[(\d+),(\d+)\]\[(\d+),(\d+)\]"/).map(Number);
  run('shell', 'input', 'tap', String(Math.round((x1+x2)/2)), String(Math.round((y1+y2)/2)));
  await wait(350);
}
const checks = [];
const record = (name, detail) => checks.push({ name, detail });
async function layout() {
  return evaluate(`(() => {
    const visible = n => n.getClientRects().length > 0;
    const overflowing = [...document.querySelectorAll('.settings-dialog *, .sidebar *, .toolbar *')]
      .filter(visible).filter(n => { const r=n.getBoundingClientRect(); return r.left < -1 || r.right > innerWidth+1; })
      .map(n => n.id || n.className || n.tagName);
    return {width:innerWidth,height:innerHeight,overflowing,screen:document.querySelector('.settings-dialog').dataset.screen,
      settingsOpen:!document.querySelector('#settings-overlay').classList.contains('hidden')};
  })()`);
}
async function keyboard(selector, name, resetFocus = true) {
  // Reset focus left by programmatic navigation before a real native touch.
  await evaluate(`${resetFocus ? 'document.activeElement?.blur();' : ''} document.querySelector(${JSON.stringify(selector)}).scrollIntoView({block:'center'})`);
  await wait(1000);
  run('shell', 'uiautomator', 'dump', '/sdcard/mobile-test.xml');
  const xml = run('shell', 'cat', '/sdcard/mobile-test.xml').toString();
  const nativeTop = Number(xml.match(/class="android.webkit.WebView"[^>]*bounds="\[\d+,(\d+)\]/)[1]);
  const before = await layout();
  const rect = await evaluate(`(() => {const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2,dpr:devicePixelRatio};})()`);
  console.log(JSON.stringify({ name, rect, nativeTop }));
  // Native touch also reopens an IME dismissed while the input retained focus.
  const x = String(Math.round(rect.x * rect.dpr)), y = String(Math.round(rect.y * rect.dpr + nativeTop));
  run('shell', 'input', 'tap', x, y);
  let shown = false;
  const deadline = Date.now() + 5000;
  while (Date.now() < deadline) {
    shown = /mInputShown=true/.test(run('shell', 'dumpsys', 'input_method').toString());
    if (shown) break;
    await wait(100);
  }
  await wait(500);
  screenshot(name);
  assert(shown, `${name}: Android IME did not open`);
  const after = await evaluate(`(() => {const n=document.querySelector(${JSON.stringify(selector)}),r=n.getBoundingClientRect();return {height:innerHeight,top:r.top,bottom:r.bottom,focused:n===document.activeElement};})()`);
  assert(after.focused, name);
  assert(after.height < before.height - 150, `${name}: WebView failed to resize: ${JSON.stringify({before,after})}`);
  assert(after.top >= 0 && after.bottom <= after.height, `${name}: focused input is obscured: ${JSON.stringify(after)}`);
  record(name, { beforeHeight: before.height, ...after });
  run('shell', 'input', 'keyevent', '4'); await wait(450);
}
const snapshot = () => evaluate(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.theme_snapshot)`);
async function editSize(value) {
  await evaluate(`(() => {const input=document.querySelector('#reading-editor [data-theme-field="typography.read_size"]');input.value=${value};input.dispatchEvent(new Event('change',{bubbles:true}));})()`);
  await wait(350);
}
try {
  // The inspector endpoint is available before scripts and asynchronous boot.
  let ready = false;
  for (let i = 0; i < 100; i++) {
    ready = await evaluate(`document.readyState==='complete' && typeof I18N!=='undefined' && !!document.querySelector('#btn-settings')?.onclick`);
    if (ready) break;
    await wait(100);
  }
  assert(ready, 'Application scripts and command bindings did not finish booting');
  await evaluate(`window.__TAURI__.core.invoke('set_ui_locale',{locale:'zh-CN'})`);
  await evaluate(`I18N.setLocale('zh-CN'); I18N.applyStaticI18n()`);
  // Real navigation establishes Android WebView touch focus on a fresh launch.
  await nativeButton('订阅');
  const feed = await evaluate(`(() => {const r=document.querySelector('#add-row').getBoundingClientRect();return {top:r.top,bottom:r.bottom,label:document.querySelector('label[for="add-url"]').textContent};})()`);
  assert(feed.top < 160 && feed.label); record('subscription-entry-at-top', feed);
  const targets = await evaluate(`['add-ok','btn-add','btn-new-folder'].map(id=>{const r=document.getElementById(id).getBoundingClientRect();return {id,width:r.width,height:r.height};})`);
  assert(targets.every(r=>r.width>=43.9&&r.height>=43.9), `Subscription touch targets: ${JSON.stringify(targets)}`);
  record('subscription-touch-targets', targets);
  screenshot('01-subscriptions');
  // Match the reported entry path: native Add establishes the input connection.
  await nativeButton('添加订阅');
  assert.equal(await evaluate(`document.activeElement.id`), 'add-url');
  await keyboard('#add-url', '02-feed-keyboard', false);
  await click('#btn-add');
  assert.equal(await evaluate(`document.activeElement.id`), 'add-url');
  await click('[data-mpage-btn="settings"]');
  assert.equal((await layout()).screen, 'home'); screenshot('03-settings-home');
  assert.equal(await evaluate(`document.querySelector('#tab-mcp').getClientRects().length`), 0);
  await click('#tab-reading');
  assert.equal((await layout()).screen, 'detail');
  assert.equal(await evaluate(`document.querySelector('#reading-editor .theme-advanced').open`), false);
  assert.equal(await evaluate(`document.querySelector('#reading-editor [data-theme-field="reader.layout"]')`), null);
  screenshot('04-reading');
  await keyboard('#reading-editor [data-theme-field="typography.read_size"]', '05-reading-keyboard');
  await click('#reading-editor .theme-advanced summary');
  await keyboard('#reading-editor [data-theme-field="typography.read_family"]', '05b-font-keyboard');
  await click('#reading-editor .theme-advanced summary');
  const initial = await snapshot(); const oldSize = initial.light.typography.read_size;
  await click('#reading-editor .theme-editor-actions button:nth-child(2)');
  assert.equal(await evaluate(`document.querySelector('#reading-editor .theme-preview-details').open`), true);
  assert.equal((await snapshot()).config.revision, initial.config.revision);
  await editSize(oldSize === 18 ? 19 : 18);
  await click('#reading-editor .theme-editor-actions button:last-child');
  assert.equal(Number(await evaluate(`document.querySelector('#reading-editor [data-theme-field="typography.read_size"]').value`)), oldSize);
  assert.equal((await snapshot()).config.revision, initial.config.revision);
  await editSize(oldSize === 18 ? 19 : 18);
  await click('#reading-editor .theme-editor-actions button:first-child');
  const saved = await snapshot(); assert(saved.config.revision > initial.config.revision);
  record('theme-save-and-discard', { beforeRevision: initial.config.revision, savedRevision: saved.config.revision });
  await editSize(20);
  await click('#settings-close');
  await click('[data-mpage-btn="settings"]'); await click('#tab-reading');
  assert.equal(Number(await evaluate(`document.querySelector('#reading-editor [data-theme-field="typography.read_size"]').value`)), saved.light.typography.read_size);
  run('shell', 'input', 'keyevent', '4'); await wait(450);
  assert.equal((await layout()).screen, 'home'); assert((await layout()).settingsOpen);
  record('android-back-detail-to-home', await layout());
  await click('#tab-ai'); screenshot('06-ai-settings');
  await keyboard('#ai-base-url', '07-ai-endpoint-keyboard');
  await keyboard('#ai-key', '08-ai-key-keyboard');
  await click('#m-settings-back'); assert.equal((await layout()).screen, 'home');
  run('shell', 'input', 'keyevent', '4'); await wait(450);
  assert.equal((await layout()).settingsOpen, false); record('android-back-home-closes-settings', await layout());
  for (const width of [360, 412]) {
    await call('Emulation.setDeviceMetricsOverride', { width, height: 867, deviceScaleFactor: 2.625, mobile: true });
    await wait(450);
    assert.equal((await layout()).width, width);
    await click('[data-mpage-btn="settings"]');
    for (const pane of ['appearance', 'reading', 'subscriptions', 'ai', 'data', 'general']) {
      await click('#tab-' + pane);
      const l = await layout(); assert.deepEqual(l.overflowing, [], `${pane} overflow at ${l.width}px`);
      if (pane === 'data') assert.equal(await evaluate(`document.querySelector('#act-backup-db').getClientRects().length`), 0);
      record(`${pane}-${l.width}px`, l); await click('#m-settings-back');
    }
    await click('#settings-close');
  }
  await call('Emulation.clearDeviceMetricsOverride', {});
  await evaluate(`I18N.setLocale('en'); I18N.applyStaticI18n()`);
  await click('[data-mpage-btn="settings"]'); await click('#tab-reading');
  assert.deepEqual((await layout()).overflowing, []); screenshot('09-reading-en');
  await click('#settings-close');
  writeFileSync(`${out}/results.json`, JSON.stringify({ serial, checks }, null, 2) + '\n');
  console.log(JSON.stringify({ out, checks: checks.length }));
} finally {
  await call('Emulation.clearDeviceMetricsOverride', {}); ws.close();
}
