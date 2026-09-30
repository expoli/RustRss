// Installed APK verification on the task-owned RustRssT5 AVD only.
// ANDROID_SERIAL=emulator-5584 CDP_URL=http://127.0.0.1:9235/json node scripts/verify-reading-experience-settings-android.mjs OUT
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

assert.equal(process.env.ANDROID_SERIAL, 'emulator-5584');
const out = resolve(process.argv[2]);
mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', process.env.ANDROID_SERIAL, ...args], { timeout: 30000 });
const pause = ms => new Promise(done => setTimeout(done, ms));
const target = (await (await fetch(process.env.CDP_URL)).json()).find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target);
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((done, fail) => { socket.onopen = done; socket.onerror = fail; });
let seq = 0;
const pending = new Map();
socket.onmessage = event => {
  const message = JSON.parse(event.data), slot = pending.get(message.id);
  if (!slot) return;
  clearTimeout(slot.timer); pending.delete(message.id);
  message.error ? slot.fail(message.error) : slot.done(message.result);
};
const call = (method, params = {}) => new Promise((done, fail) => {
  const id = ++seq, timer = setTimeout(() => { pending.delete(id); fail(new Error(`CDP timeout ${method}`)); }, 20000);
  pending.set(id, { done, fail, timer });
  socket.send(JSON.stringify({ id, method, params }));
});
async function js(expression) {
  const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  assert(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
  return result.result.value;
}
async function until(expression) {
  for (let i = 0; i < 100; i++) {
    try { if (await js(expression)) return; } catch { /* reload */ }
    await pause(120);
  }
  throw new Error(`Timed out: ${expression}`);
}
async function native(selector) {
  const pos = await js(`(() => {const n=document.querySelector(${JSON.stringify(selector)});if(!n)return null;n.scrollIntoView({block:'center'});const r=n.getBoundingClientRect();const x=r.x+r.width/2,y=r.y+r.height/2;return {x,y,w:r.width,h:r.height,dpr:devicePixelRatio,visible:r.width>0&&r.height>0&&y>=0&&y<visualViewport.height&&document.elementFromPoint(x,y)?.closest(${JSON.stringify(selector)})===n}})()`);
  assert(pos?.visible, `${selector}: ${JSON.stringify(pos)}`);
  adb('shell', 'input', 'tap', String(Math.round(pos.x * pos.dpr)), String(Math.round(JSON.parse(target.description).screenY + pos.y * pos.dpr)));
  await pause(350);
}
const snap = name => writeFileSync(`${out}/${name}.png`, adb('exec-out', 'screencap', '-p'));
const record = [];
function check(name, value, detail = {}) { assert(value, `${name}: ${JSON.stringify(detail)}`); record.push({ name, ...detail }); }
const core = (cmd, args = {}) => js(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
const setting = () => core('get_ui_settings');
const visible = selector => js(`(() => {const n=document.querySelector(${JSON.stringify(selector)});return !!n&&n.getClientRects().length>0})()`);
const value = selector => js(`document.querySelector(${JSON.stringify(selector)})?.value`);
const set = (selector, next) => js(`(() => {const n=document.querySelector(${JSON.stringify(selector)});n.value=${JSON.stringify(next)};n.dispatchEvent(new Event('change',{bubbles:true}));return n.value})()`);
const click = selector => js(`document.querySelector(${JSON.stringify(selector)}).click()`);
try {
  await until(`!!document.querySelector('#m-nav [data-mpage-btn="settings"]')`);
  if (await js(`document.body.dataset.mpage==='settings'`)) await native('#m-nav [data-mpage-btn="articles"]');
  await native('#m-nav [data-mpage-btn="settings"]');
  await until(`document.querySelector('.settings-dialog').dataset.screen==='home'`);
  const categories = ['appearance', 'reading', 'subscriptions', 'ai', 'data', 'general'];
  for (const name of categories) {
    check(`android-category-${name}`, await visible(`#tab-${name}`), { summary: await js(`document.querySelector('#settings-summary-${name}')?.textContent`) });
    await native(`#tab-${name}`);
    await until(`document.querySelector('.settings-dialog').dataset.screen==='detail'`);
    check(`android-pane-${name}`, await visible(`#pane-${name}`), { title: await js(`document.querySelector('#m-settings-title').textContent`) });
    snap(`category-${name}`);
    adb('shell', 'input', 'keyevent', 'KEYCODE_BACK');
    await until(`document.querySelector('.settings-dialog').dataset.screen==='home'`);
  }
  check('android-mcp-hidden', !await visible('#tab-mcp'));
  await native('#tab-reading');
  check('android-mark-read-reachable', await visible('#set-mark-read'));
  check('android-no-desktop-reading-layout', !await visible('#reading-editor [data-theme-field="reader.layout"]'));
  adb('shell', 'input', 'keyevent', 'KEYCODE_BACK');
  await until(`document.querySelector('.settings-dialog').dataset.screen==='home'`);
  await native('#tab-data');
  check('android-opml-subscriptions-only', await visible('#pane-data .m-only') && !await visible('#act-backup-db') && !await visible('#act-restore-db'), { hint: await js(`document.querySelector('#pane-data .m-only').textContent`) });
  adb('shell', 'input', 'keyevent', 'KEYCODE_BACK');
  await until(`document.querySelector('.settings-dialog').dataset.screen==='home'`);
  await native('#tab-appearance');
  const start = await setting();
  const revision = start.theme_snapshot.config.revision;
  const size = start.theme_snapshot.light.typography.ui_size;
  const candidate = size === 17 ? 18 : 17;
  await set('#appearance-editor [data-theme-field="typography.ui_size"]', candidate);
  await until(`!document.querySelector('#appearance-editor .theme-editor-actions button').disabled`);
  check('theme-draft-no-write', (await setting()).theme_snapshot.config.revision === revision, { revision, candidate });
  snap('appearance-draft');
  adb('shell', 'input', 'keyevent', 'KEYCODE_BACK');
  await until(`document.querySelector('.settings-dialog').dataset.screen==='home'`);
  await native('#tab-appearance');
  check('detail-back-retains-draft', await value('#appearance-editor [data-theme-field="typography.ui_size"]') == candidate);
  await native('#m-nav [data-mpage-btn="articles"]');
  await until(`document.body.dataset.mpage==='articles'`);
  check('destination-discard-no-write', (await setting()).theme_snapshot.config.revision === revision);
  await native('#m-nav [data-mpage-btn="settings"]');
  await native('#tab-appearance');
  check('destination-reopen-saved-value', await value('#appearance-editor [data-theme-field="typography.ui_size"]') == size, { size });
  check('android-no-desktop-appearance-width', !await visible('#appearance-editor [data-theme-field="chrome.sidebar_width"]'));
  // All persistence below operates only on this synthetic fixture.
  await set('#appearance-editor [data-theme-field="typography.ui_size"]', candidate);
  await click('#appearance-editor .theme-editor-actions button:first-child');
  await until(`document.querySelector('#appearance-editor .theme-editor-actions button:first-child').disabled`);
  const saved = await setting();
  check('theme-save-cas-readback', saved.theme_snapshot.config.revision === revision + 1 && saved.theme_snapshot.light.typography.ui_size === candidate, { from: revision, to: saved.theme_snapshot.config.revision });
  await click('#appearance-editor .theme-advanced summary');
  check('theme-advanced-controls', await visible('#appearance-editor [data-theme-field="chrome.radius"]') && !!await js(`document.querySelector('#appearance-editor [data-theme-field="colors.light.accent"]')`));
  check('theme-history-actions', await visible('#appearance-editor [data-theme-history]') && await visible('#appearance-editor .theme-advanced button'));
  await native('#m-settings-back');
  await native('#tab-subscriptions');
  const originalProxy = (await setting()).proxy;
  await set('#set-proxy-mode', 'custom');
  await set('#set-proxy-url', 'http://user:pass@127.0.0.1:8080');
  await click('#set-proxy-save');
  await until(`document.querySelector('#set-proxy-status').textContent.length>0`);
  check('proxy-failure-retains-input-and-links-error', await value('#set-proxy-url') === 'http://user:pass@127.0.0.1:8080' && await js(`document.querySelector('#set-proxy-url').getAttribute('aria-invalid')`) === 'true', { status: await js(`document.querySelector('#set-proxy-status').textContent`) });
  check('proxy-failure-no-write', JSON.stringify((await setting()).proxy) === JSON.stringify(originalProxy));
  await set('#set-proxy-mode', 'direct');
  await click('#set-proxy-save');
  await until(`!document.querySelector('#set-proxy-save').disabled`);
  check('proxy-save-readback', (await setting()).proxy.mode === 'direct');
  await native('#m-settings-back');
  await native('#tab-ai');
  const providers = ['ollama', 'openai', 'anthropic', 'gemini'];
  const aiOptions = await js(`[...document.querySelector('#ai-provider').options].map(o=>o.value)`);
  check('four-ai-providers', JSON.stringify(aiOptions) === JSON.stringify(providers), { aiOptions });
  await set('#ai-provider', 'ollama');
  await set('#ai-model', 'synthetic-t5');
  await set('#ai-base-url', 'http://10.0.2.2:18080');
  await click('#ai-save');
  await until(`document.querySelector('#ai-status').textContent.includes('Saved') || document.querySelector('#ai-status').textContent.includes('已保存')`);
  check('ai-settings-save-readback', (await core('get_ai_settings')).model === 'synthetic-t5');
  check('ai-advanced-and-key-controls', await visible('#ai-key') && await visible('#ai-save') && await visible('#ai-clear-key') && await visible('#ai-test') && !!await js(`document.querySelector('#ai-max-tokens')`));
  snap('ai-settings');
  await native('#m-settings-back');
  await native('#tab-general');
  check('general-locale-about-log', await visible('#set-language') && await visible('#about-open-source') && await visible('#set-log-level') && !await visible('#act-open-logs'));
  await native('#m-settings-back');
  adb('shell', 'input', 'keyevent', 'KEYCODE_BACK');
  await until(`document.body.dataset.mpage==='articles'`);
  record.push({ name: 'artifact', apkSha256: process.env.APK_SHA256 });
  writeFileSync(`${out}/android-results.json`, JSON.stringify(record, null, 2) + '\n');
  console.log(JSON.stringify({ checks: record.length, out }));
} finally { socket.close(); }
