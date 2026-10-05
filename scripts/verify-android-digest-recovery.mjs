// Rebuilt debug APK on an isolated emulator. Uses real IPC for unconfigured AI
// path, then a reload-persistent IPC fixture (no AI calls / no digest:done events).
// adb forward tcp:9228 localabstract:webview_devtools_remote_<app-pid>
// ANDROID_SERIAL=emulator-5582 node scripts/verify-android-digest-recovery.mjs <evidence-dir>
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
const serial = process.env.ANDROID_SERIAL;
assert(serial, 'Use an isolated emulator');
const out = resolve(process.argv[2] || '/tmp/rustrss-digest-recovery');
mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync(process.env.ADB || '/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, ...args], { timeout: 20000 });
const wait = ms => new Promise(r => setTimeout(r, ms));
const tabs = await (await fetch(process.env.CDP_URL || 'http://127.0.0.1:9228/json')).json();
const ws = new WebSocket(tabs.find(t => t.type === 'page').webSocketDebuggerUrl);
await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
let seq = 0; const pending = new Map();
ws.onmessage = event => {
  const msg = JSON.parse(event.data), p = pending.get(msg.id);
  if (p) { pending.delete(msg.id); msg.error ? p.reject(msg.error) : p.resolve(msg.result); }
};
function call(method, params = {}) {
  const id = ++seq;
  return new Promise((resolve, reject) => { pending.set(id, { resolve, reject }); ws.send(JSON.stringify({ id, method, params })); });
}
async function evaluate(expression) {
  const r = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  assert(!r.exceptionDetails, JSON.stringify(r.exceptionDetails)); return r.result.value;
}
async function until(expression) {
  for (let i = 0; i < 60; i++) { const value = await evaluate(expression); if (value) return value; await wait(200); }
  throw Error(`Timed out: ${expression}`);
}
const screenshot = name => writeFileSync(`${out}/${name}.png`, adb('exec-out', 'screencap', '-p'));
const checks = [];
let injection;
try {
  await call('Page.enable');
  // First exercise the actual rebuilt command/error path without AI configuration.
  await evaluate(`RustRssDigestBridge.openDate(RustRssDigestBridge.date('today'))`);
  await until(`!!document.querySelector('.digest-gen')`);
  await evaluate(`document.querySelector('.digest-gen').click()`);
  const realError = await until(`document.querySelector('#status').classList.contains('error') && document.querySelector('#status').textContent`);
  await until(`!document.querySelector('#digest-progress')`);
  checks.push({ name: 'real unconfigured-AI generation resets pending state', error: realError });

  const fixtureScript = `addEventListener('DOMContentLoaded', () => {
    window.__digestRecoveryFixtureInstalled = true;
    const real = window.__TAURI__;
    window.__TAURI__ = { ...real, core: { ...real.core, invoke: async (cmd, args) => {
      const fixture = JSON.parse(sessionStorage.getItem('digest.recovery.fixture') || '{}');
      if (cmd === 'digest_generate' && args.date === '2026-10-04') {
        fixture.active = true; sessionStorage.setItem('digest.recovery.fixture', JSON.stringify(fixture));
        return { job_id: 'reload-fixture-job' };
      }
      if (['digest_get', 'digest_status'].includes(cmd) && args.date === '2026-10-04') {
        if (cmd === 'digest_status') {
          fixture.polls = [...(fixture.polls || []), Date.now()];
          sessionStorage.setItem('digest.recovery.fixture', JSON.stringify(fixture));
        }
        const status = { generating: !!fixture.active, has_report: !!fixture.finished, candidate_count: 2, added: 0, changed: 0, removed: 0 };
        if (cmd === 'digest_status') return status;
        return { date: args.date, scope_key: args.scopeKey || 'all', generating: !!fixture.active, has_report: !!fixture.finished,
          status, sections: fixture.finished ? [{ title: 'Recovered fixture report', text: 'Completed without any digest:done event.' }] : [],
          overview: '', markdown: '', article_count: 2, cache_hits: 0 };
      }
      return real.core.invoke(cmd, args);
    } } };
  }, { once: true });`;
  injection = (await call('Page.addScriptToEvaluateOnNewDocument', { source: fixtureScript })).identifier;
  await evaluate(`sessionStorage.removeItem('ui.location'); sessionStorage.setItem('digest.recovery.fixture', JSON.stringify({active:false,polls:[]}))`);
  await call('Page.reload'); await until(`window.__digestRecoveryFixtureInstalled && !!window.RustRssLocation`); await wait(2500);
  await evaluate(`RustRssDigestBridge.openDate('2026-10-04', 'all')`);
  await evaluate(`document.querySelector('.digest-gen').click()`);
  await until(`!!document.querySelector('#digest-progress')`);
  const before = await evaluate(`JSON.parse(sessionStorage.getItem('ui.location'))`);
  assert.equal(before.mpage, 'digest'); assert.equal(before.digestDate, '2026-10-04');
  screenshot('01-generating');

  adb('shell', 'input', 'keyevent', 'KEYCODE_HOME'); await wait(1500);
  adb('shell', 'monkey', '-p', 'tech.expoli.rustrss', '-c', 'android.intent.category.LAUNCHER', '1'); await wait(1000);
  assert(await evaluate(`!!document.querySelector('#digest-progress')`));
  // Force page memory loss: warm WebView itself often survives HOME intact.
  await call('Page.reload');
  await until(`!!document.querySelector('#digest-progress') && document.querySelector('#digest-cancel').disabled`);
  assert.equal(await evaluate(`document.body.dataset.mpage`), 'reader');
  screenshot('02-reloaded-generating');
  await evaluate(`{ const f=JSON.parse(sessionStorage.getItem('digest.recovery.fixture')); f.polls=[]; sessionStorage.setItem('digest.recovery.fixture',JSON.stringify(f)); }`);
  const polls = await until(`JSON.parse(sessionStorage.getItem('digest.recovery.fixture')).polls.length >= 2 && JSON.parse(sessionStorage.getItem('digest.recovery.fixture')).polls`);
  assert(polls[1] - polls[0] >= 2900, JSON.stringify(polls));
  checks.push({ name: 'HOME return + forced reload restores generating placeholder and 3s polling', location: before, pollIntervalMs: polls[1] - polls[0] });
  await evaluate(`{ const f=JSON.parse(sessionStorage.getItem('digest.recovery.fixture'));f.active=false;f.finished=true;sessionStorage.setItem('digest.recovery.fixture',JSON.stringify(f)); }`);
  await until(`document.querySelector('#reader').textContent.includes('Recovered fixture report') && !document.querySelector('#digest-progress')`);
  screenshot('03-completed-without-event');
  checks.push({ name: 'completion refreshes cached report without digest:done', scope: await evaluate(`JSON.parse(sessionStorage.getItem('ui.location')).digestScope`) });

  const elapsed = await evaluate(`JSON.parse(sessionStorage.getItem('digest.recovery.fixture')).polls.length`);
  await wait(3500);
  assert.equal(await evaluate(`JSON.parse(sessionStorage.getItem('digest.recovery.fixture')).polls.length`), elapsed);
  checks.push({ name: 'completion stops polling' });
  // Back returns home once: repeated completion renders must not stack reader history.
  await evaluate(`document.querySelector('#m-reader-back').click()`);
  await until(`document.body.dataset.mpage === 'digest' && !!document.querySelector('.m-digest-home')`);
  checks.push({ name: 'reader back returns to digest home after completion' });
  writeFileSync(`${out}/result.json`, JSON.stringify({ serial, checks, mockScope: 'Only digest IPC is mocked; fixture state survives reload. Actual configuration-error path uses Rust IPC. No real AI service, process-kill, or vendor ROM validation.' }, null, 2) + '\n');
  console.log(JSON.stringify(checks, null, 2));
} finally {
  if (injection) await call('Page.removeScriptToEvaluateOnNewDocument', { identifier: injection });
  await evaluate(`sessionStorage.removeItem('digest.recovery.fixture'); sessionStorage.removeItem('ui.location')`);
  await call('Page.reload'); ws.close();
}
