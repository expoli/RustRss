// Additional installed-APK settings checks on the task-owned synthetic AVD.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

assert.equal(process.env.ANDROID_SERIAL, 'emulator-5584');
const out = resolve(process.argv[2]); mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', process.env.ANDROID_SERIAL, ...args], { timeout: 30000 });
const pause = ms => new Promise(done => setTimeout(done, ms));
let target = (await (await fetch(process.env.CDP_URL)).json()).find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target);
let socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((done, fail) => { socket.onopen = done; socket.onerror = fail; });
let seq = 0; const pending = new Map();
function receive(event) { const m = JSON.parse(event.data), slot = pending.get(m.id); if (!slot) return; clearTimeout(slot.timer); pending.delete(m.id); m.error ? slot.fail(m.error) : slot.done(m.result); }
socket.onmessage = receive;
const call = (method, params = {}) => new Promise((done, fail) => { const id = ++seq, timer = setTimeout(() => { pending.delete(id); fail(Error(`CDP timeout ${method}`)); }, 20000); pending.set(id, { done, fail, timer }); socket.send(JSON.stringify({ id, method, params })); });
async function js(expression) { const r = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true }); assert(!r.exceptionDetails, JSON.stringify(r.exceptionDetails)); return r.result.value; }
async function reconnectAfterReload() {
  await js('location.reload();true');
  socket.close();
  await pause(800);
  target = (await (await fetch(process.env.CDP_URL)).json()).find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
  assert(target, 'Reloaded WebView target missing');
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((done, fail) => { socket.onopen = done; socket.onerror = fail; });
  socket.onmessage = receive;
  await until(`!!document.querySelector('#m-nav [data-mpage-btn="settings"]')`);
  await pause(800);
}
async function until(expression) { for (let i = 0; i < 100; i++) { try { if (await js(expression)) return; } catch { /* reload */ } await pause(120); } throw Error(`Timed out: ${expression}`); }
const click = selector => js(`document.querySelector(${JSON.stringify(selector)}).click()`);
const set = (selector, v) => js(`(() => {const n=document.querySelector(${JSON.stringify(selector)});n.value=${JSON.stringify(v)};n.dispatchEvent(new Event('change',{bubbles:true}));return n.value})()`);
const core = (cmd, args = {}) => js(`window.__TAURI__.core.invoke(${JSON.stringify(cmd)},${JSON.stringify(args)})`);
const setting = () => core('get_ui_settings');
const visible = selector => js(`!!document.querySelector(${JSON.stringify(selector)})?.getClientRects().length`);
const shot = name => writeFileSync(`${out}/${name}.png`, adb('exec-out', 'screencap', '-p'));
const checks = []; const check = (name, pass, data = {}) => { assert(pass, `${name}: ${JSON.stringify(data)}`); checks.push({ name, ...data }); };
async function pane(name) {
  if (await js(`document.body.dataset.mpage!=='settings'`)) await click('#m-nav [data-mpage-btn="settings"]');
  if (await js(`document.querySelector('.settings-dialog').dataset.screen==='detail'`)) await click('#m-settings-back');
  await click(`#tab-${name}`);
  await until(`document.querySelector('#pane-${name}').getClientRects().length>0`);
}
async function saveTheme(host) {
  const before = (await setting()).theme_snapshot.config.revision;
  await click(`${host} .theme-editor-actions button:first-child`);
  await until(`document.querySelector(${JSON.stringify(host + ' .theme-editor-actions button:first-child')}).disabled`);
  const after = await setting();
  check('theme-revision-increments', after.theme_snapshot.config.revision === before + 1, { before, after: after.theme_snapshot.config.revision });
  return after;
}
try {
  await reconnectAfterReload();
  await pane('appearance');
  const base = await setting();
  const originalRev = base.theme_snapshot.config.revision;
  const nextMode = base.theme_snapshot.config.mode === 'dark' ? 'light' : 'dark';
  const nextPreset = base.theme_snapshot.config.light_preset === 'paper' ? 'slate' : 'paper';
  const nextSummary = base.theme_snapshot.light.list.summary_lines === 1 ? 2 : 1;
  await set('#appearance-editor [data-theme-top="mode"]', nextMode);
  await click(`#appearance-editor [data-variant="light"][data-preset="${nextPreset}"]`);
  await set('#appearance-editor [data-theme-field="list.summary_lines"]', nextSummary);
  await click('#appearance-editor .theme-editor-actions button:nth-child(2)');
  check('theme-preview-does-not-persist', (await setting()).theme_snapshot.config.revision === originalRev);
  let saved = await saveTheme('#appearance-editor');
  check('theme-mode-preset-summary-readback', saved.theme_snapshot.config.mode === nextMode && saved.theme_snapshot.config.light_preset === nextPreset && saved.theme_snapshot.light.list.summary_lines === nextSummary, { mode: saved.theme_snapshot.config.mode, preset: saved.theme_snapshot.config.light_preset });
  await js(`document.querySelector('#appearance-editor .theme-advanced').open=true;true`);
  await click('#appearance-editor [data-theme-history] + button + button');
  await until(`[...document.querySelector('#appearance-editor [data-theme-history]').options].some(o=>o.value===${JSON.stringify(String(originalRev))})`);
  const history = await js(`[...document.querySelector('#appearance-editor [data-theme-history]').options].map(o=>o.value)`);
  check('theme-history-refresh-has-prior-revision', history.includes(String(originalRev)), { history });
  const historyBefore = saved.theme_snapshot.config.revision;
  await set('#appearance-editor [data-theme-history]', String(originalRev));
  await click('#appearance-editor [data-theme-history] + button');
  await until(`document.querySelector('#appearance-editor .theme-editor-status').textContent.includes('Saved') || document.querySelector('#appearance-editor .theme-editor-status').textContent.includes('已保存')`);
  saved = await setting();
  check('theme-history-restore-new-revision', saved.theme_snapshot.config.revision === historyBefore + 1 && saved.theme_snapshot.config.light_preset === base.theme_snapshot.config.light_preset, { from: historyBefore, to: saved.theme_snapshot.config.revision });
  // A live external theme writer invalidates the editor's stale CAS revision.
  await set('#appearance-editor [data-theme-field="list.summary_lines"]', 3);
  const staleBase = (await setting()).theme_snapshot.config.revision;
  const externalMode = saved.theme_snapshot.config.mode === 'dark' ? 'light' : 'dark';
  await core('update_ui_theme', { expectedRevision: staleBase, patch: { mode: externalMode } });
  await click('#appearance-editor .theme-editor-actions button:first-child');
  await until(`/changed|修改|冲突|失败|failed|conflict/i.test(document.querySelector('#appearance-editor .theme-editor-status').textContent)`);
  const afterConflict = await setting();
  check('theme-stale-cas-does-not-write', afterConflict.theme_snapshot.config.revision === staleBase + 1 && afterConflict.theme_snapshot.config.mode === externalMode, { revision: afterConflict.theme_snapshot.config.revision });
  await click('#appearance-editor .theme-editor-actions button:last-child');
  check('theme-discard-after-cas', await js(`document.querySelector('#appearance-editor .theme-editor-actions button:first-child').disabled`));
  await reconnectAfterReload();
  await pane('reading');
  const readInitial = await setting();
  const readSize = readInitial.theme_snapshot.light.typography.read_size;
  const nextSize = readSize === 18 ? 19 : 18;
  await set('#reading-editor [data-theme-field="typography.read_size"]', nextSize);
  await set('#reading-editor [data-theme-field="typography.line_height"]', 1.8);
  await set('#reading-editor [data-theme-field="reader.paragraph_gap"]', 1.4);
  const readingSaved = await saveTheme('#reading-editor');
  check('reading-typography-readback', readingSaved.theme_snapshot.light.typography.read_size === nextSize && Math.abs(readingSaved.theme_snapshot.light.typography.line_height - 1.8) < .01 && Math.abs(readingSaved.theme_snapshot.light.reader.paragraph_gap - 1.4) < .01, { size: readingSaved.theme_snapshot.light.typography.read_size });
  await pane('ai');
  for (const provider of ['ollama', 'openai', 'anthropic', 'gemini']) {
    await set('#ai-provider', provider);
    await set('#ai-model', 'synthetic-t5-' + provider);
    await set('#ai-base-url', 'http://10.0.2.2:18080');
    await click('#ai-save');
    await until(`document.querySelector('#ai-status').textContent.includes('Saved') || document.querySelector('#ai-status').textContent.includes('已保存')`);
    const actual = await core('get_ai_settings');
    check('ai-provider-save-' + provider, actual.provider === provider && actual.model === 'synthetic-t5-' + provider, { provider: actual.provider, model: actual.model });
  }
  await set('#ai-key', 'synthetic-t5-key-do-not-use');
  await click('#ai-save');
  await until(`document.querySelector('#ai-key').value===''`);
  const keyState = await core('get_ai_settings');
  check('android-keystore-key-state', keyState.has_key === true && !await js(`document.querySelector('#ai-key').value`), { source: keyState.key_source });
  await click('#ai-clear-key');
  await until(`document.querySelector('#ai-status').textContent.includes('cleared') || document.querySelector('#ai-status').textContent.includes('清除')`);
  check('android-keystore-key-clear', !(await core('get_ai_settings')).has_key);
  await pane('subscriptions');
  const mirrorBefore = (await setting()).rsshub_mirror;
  await set('#set-rsshub-mirror', 'http://10.0.2.2:18080');
  await click('#btn-rsshub-save');
  await until(`document.querySelector('#rsshub-status').textContent.length>1`);
  check('rsshub-mirror-save', (await setting()).rsshub_mirror === 'http://10.0.2.2:18080');
  await click('#btn-rsshub-migrate');
  await until(`document.querySelector('#rsshub-status').textContent.length>1`);
  check('rsshub-migration-control-reachable', await visible('#btn-rsshub-migrate'));
  await set('#set-rsshub-mirror', mirrorBefore);
  await click('#btn-rsshub-save');
  await pane('data');
  check('android-saf-actions-reachable', await visible('#act-import-opml') && await visible('#act-export-opml'));
  // Verify platform gating after a wide landscape viewport; always restore the AVD display.
  adb('shell', 'settings', 'put', 'system', 'accelerometer_rotation', '0');
  adb('shell', 'settings', 'put', 'system', 'user_rotation', '1');
  adb('shell', 'wm', 'size', '2560x1600');
  adb('shell', 'wm', 'density', '160');
  await pause(1500);
  const wide = await js(`({width:innerWidth,orientation:screen.orientation.type,android:document.body.dataset.android,mcp:!!document.querySelector('#tab-mcp').getClientRects().length,backup:!!document.querySelector('#act-backup-db').getClientRects().length,hint:!!document.querySelector('#pane-data .m-only').getClientRects().length})`);
  check('resized-android-capabilities', wide.android === '1' && !wide.mcp && !wide.backup && wide.hint, wide);
  shot('wide-android-data');
  adb('shell', 'wm', 'size', 'reset');
  adb('shell', 'wm', 'density', 'reset');
  adb('shell', 'settings', 'put', 'system', 'accelerometer_rotation', '1');
  await pause(1000);
  checks.push({ name: 'artifact', apkSha256: process.env.APK_SHA256 });
  writeFileSync(`${out}/android-followup-results.json`, JSON.stringify(checks, null, 2) + '\n');
  console.log(JSON.stringify({ checks: checks.length, out }));
} finally {
  adb('shell', 'wm', 'size', 'reset'); adb('shell', 'wm', 'density', 'reset'); adb('shell', 'settings', 'put', 'system', 'accelerometer_rotation', '1');
  socket.close();
}
