// Final-APK native Android settings side effects that are separate from category navigation.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

assert.equal(process.env.ANDROID_SERIAL, 'emulator-5584');
const out = resolve(process.argv[2]); mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', process.env.ANDROID_SERIAL, ...args], { timeout: 30000 });
const wait = ms => new Promise(done => setTimeout(done, ms));
const requests = [];
const server = createServer((req, res) => {
  requests.push({ method: req.method, path: req.url?.split('?')[0] });
  res.setHeader('content-type', 'application/json');
  if (req.url === '/api/generate') res.end(JSON.stringify({ response: '可用' }));
  else res.end(JSON.stringify({ version: 't5-synthetic' }));
});
await new Promise(done => server.listen(0, '0.0.0.0', done));
const endpoint = `http://10.0.2.2:${server.address().port}`;
const target = (await (await fetch(process.env.CDP_URL)).json()).find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target);
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((done, fail) => { socket.onopen = done; socket.onerror = fail; });
let seq = 0; const pending = new Map();
socket.onmessage = event => { const m = JSON.parse(event.data), p = pending.get(m.id); if (!p) return; clearTimeout(p.timer); pending.delete(m.id); m.error ? p.fail(m.error) : p.done(m.result); };
const js = expression => new Promise((done, fail) => {
  const id = ++seq, timer = setTimeout(() => { pending.delete(id); fail(Error(`CDP timeout: ${expression.slice(0,80)}`)); }, 15000);
  pending.set(id, { timer, done: r => { if (r.exceptionDetails) fail(Error(JSON.stringify(r.exceptionDetails))); else done(r.result.value); }, fail });
  socket.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, returnByValue: true, awaitPromise: true } }));
});
async function until(expression) { for (let i = 0; i < 100; i++) { if (await js(expression)) return; await wait(120); } throw Error(`Timed out: ${expression}`); }
const core = (cmd, args = {}) => js(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
const setting = () => core('get_ui_settings');
const click = selector => js(`document.querySelector(${JSON.stringify(selector)}).click();true`);
const set = (selector, value) => js(`(()=>{const n=document.querySelector(${JSON.stringify(selector)});n.value=${JSON.stringify(value)};n.dispatchEvent(new Event('change',{bubbles:true}));return n.value})()`);
const toggle = (selector, checked) => js(`(()=>{const n=document.querySelector(${JSON.stringify(selector)});n.checked=${checked};n.dispatchEvent(new Event('change',{bubbles:true}));return n.checked})()`);
async function pane(name) {
  if (await js(`document.body.dataset.mpage!=='settings'`)) await click('#m-nav [data-mpage-btn="settings"]');
  if (await js(`document.querySelector('.settings-dialog').dataset.screen==='detail'`)) await click('#m-settings-back');
  await click(`#tab-${name}`);
  await until(`document.querySelector('#pane-${name}').getClientRects().length>0`);
}
async function dropdown(id, index) {
  await click(`#${id}`);
  await until(`document.querySelector('#ctx-menu')?.dataset.dropdown===${JSON.stringify(id)}`);
  await js(`document.querySelectorAll('#ctx-menu button')[${index}].click();true`);
  await until(`!document.querySelector('#ctx-menu')`);
}
const checks = []; const check = (name, pass, detail = {}) => { assert(pass, `${name}: ${JSON.stringify(detail)}`); checks.push({ name, ...detail }); };
try {
  // Route proof: detail -> category home -> source page.
  if (await js(`document.body.dataset.mpage==='settings'`)) await click('#m-nav [data-mpage-btn="articles"]');
  await until(`document.body.dataset.mpage==='articles'`);
  await click('#m-nav [data-mpage-btn="settings"]');
  await until(`document.body.dataset.mpage==='settings' && document.querySelector('.settings-dialog').dataset.screen==='home'`);
  await click('#tab-reading');
  await until(`document.querySelector('.settings-dialog').dataset.screen==='detail' && document.querySelector('#pane-reading').getClientRects().length>0`);
  await wait(250);
  const route = ['detail'];
  adb('shell', 'input', 'keyevent', 'KEYCODE_BACK');
  await until(`document.querySelector('.settings-dialog').dataset.screen==='home'`);
  route.push('home');
  adb('shell', 'input', 'keyevent', 'KEYCODE_BACK');
  await until(`document.body.dataset.mpage==='articles'`);
  route.push('articles');
  check('android-back-detail-home-source', route.join('>') === 'detail>home>articles', { route });

  await pane('subscriptions');
  await set('#set-proxy-mode', 'custom');
  await set('#set-proxy-url', 'http://127.0.0.1:8899');
  await set('#set-proxy-bypass', 'localhost,127.0.0.1');
  await click('#set-proxy-save');
  await until(`document.querySelector('#set-proxy-status').textContent.length>0`);
  const proxy = (await setting()).proxy;
  check('proxy-custom-bypass-readback', proxy.mode === 'custom' && proxy.url === 'http://127.0.0.1:8899' && proxy.no_proxy === 'localhost,127.0.0.1', proxy);
  await set('#set-proxy-mode', 'direct');
  await click('#set-proxy-save');
  await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.proxy.mode==='direct')`);
  await dropdown('set-refresh-interval', 1);
  await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.refresh_interval_minutes==='15')`);
  await dropdown('set-refresh-concurrency', 2);
  await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.refresh_concurrency===12)`);
  await toggle('#set-refresh-on-start', true);
  await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.refresh_on_start===true)`);
  await toggle('#set-notify-new-articles', true);
  await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.notify_new_articles===true)`);
  check('refresh-interval-concurrency-start-notify-readback', true, { interval: 15, concurrency: 12, startup: true, notify: true });
  await set('#set-rsshub-mirror', endpoint);
  await click('#btn-rsshub-save');
  await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.rsshub_mirror===${JSON.stringify(endpoint)})`);
  await click('#btn-rsshub-test');
  await until(`document.querySelector('#rsshub-status').textContent.includes('/version')`);
  check('rsshub-native-test-success', requests.some(r => r.path === '/version') && (await js(`document.querySelector('#rsshub-status').textContent`)).includes('/version'));
  await click('#btn-rsshub-migrate');
  await until(`document.querySelector('#rsshub-status').textContent.length>0`);
  check('rsshub-zero-candidate-feedback', (await core('preview_rsshub_migration')) === 0 && !await js(`!!document.querySelector('.prompt-overlay')`));

  await pane('ai');
  await set('#ai-provider', 'ollama');
  await set('#ai-model', 't5-synthetic-model');
  await set('#ai-base-url', endpoint);
  await set('#ai-target', 'fr');
  await set('#ai-max-tokens', '1024');
  await click('#ai-save');
  await until(`window.__TAURI__.core.invoke('get_ai_settings').then(s=>s.model==='t5-synthetic-model')`);
  let ai = await core('get_ai_settings');
  check('ai-endpoint-language-max-readback', ai.base_url === endpoint && ai.translate_target === 'fr' && ai.max_output_tokens === 1024,
    { provider: ai.provider, baseUrl: ai.base_url, target: ai.translate_target, max: ai.max_output_tokens });
  await dropdown('set-ai-reasoning', 4);
  await until(`window.__TAURI__.core.invoke('get_ai_settings').then(s=>s.reasoning_effort==='high')`);
  await toggle('#set-ai-confirm', false);
  await until(`window.__TAURI__.core.invoke('get_ai_settings').then(s=>s.confirm_before_send===false)`);
  await toggle('#set-ai-confirm', true);
  await until(`window.__TAURI__.core.invoke('get_ai_settings').then(s=>s.confirm_before_send===true)`);
  check('ai-reasoning-confirm-readback', true, { reasoning: 'high', confirmRestored: true });
  await click('#ai-test');
  await until(`document.querySelector('#ai-status').textContent.includes('可用') || document.querySelector('#ai-status').textContent.includes('Available')`);
  check('ai-native-test-success', requests.some(r => r.method === 'POST' && r.path === '/api/generate'), { response: await js(`document.querySelector('#ai-status').textContent`) });

  await pane('reading');
  const markInitial = (await setting()).mark_read_on_navigate;
  await toggle('#set-mark-read', !markInitial);
  await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.mark_read_on_navigate===${!markInitial})`);
  check('reading-mark-read-save-readback', (await setting()).mark_read_on_navigate === !markInitial);

  await pane('general');
  await dropdown('set-language', 1);
  await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.locale==='zh-CN')`);
  check('locale-zh-native-readback', (await js(`document.querySelector('#m-settings-title').textContent`)).includes('设置'));
  await dropdown('set-log-level', 1);
  await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.log_level==='debug')`);
  await dropdown('set-language', 2);
  await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.locale==='en')`);
  check('locale-en-native-readback', (await js(`document.querySelector('#m-settings-title').textContent`)).includes('Settings'));
  await dropdown('set-log-level', 0);
  await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.log_level==='info')`);
  const about = await js(`({license:document.querySelector('#pane-general').textContent.includes('AGPL-3.0-or-later'),privacy:document.querySelector('#pane-general').textContent.includes('Privacy'),source:!!document.querySelector('#about-open-source').getClientRects().length})`);
  check('about-license-privacy-source-native', about.license && about.privacy && about.source, about);
  check('log-level-debug-info-readback', (await setting()).log_level === 'info');
  checks.push({ name: 'artifact', apkSha256: process.env.APK_SHA256, endpointRequests: requests });
  writeFileSync(`${out}/android-required-results.json`, JSON.stringify(checks, null, 2) + '\n');
  console.log(JSON.stringify({ checks: checks.length, out }));
} finally { socket.close(); server.close(); }
