// Run immediately after verify-reading-experience-menu.mjs on the task-owned AVD.
// ANDROID_SERIAL=emulator-5582 node scripts/verify-reading-experience-menu-restart.mjs OUT
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const serial = process.env.ANDROID_SERIAL;
assert(serial, 'Select the task-owned emulator');
const out = resolve(process.argv[2]);
mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, ...args], { timeout: 30000, encoding: 'utf8' });
const delay = ms => new Promise(resolveWait => setTimeout(resolveWait, ms));
adb('shell', 'am', 'force-stop', 'tech.expoli.rustrss');
adb('shell', 'am', 'start', '-n', 'tech.expoli.rustrss/.MainActivity');
let pid;
for (let i = 0; i < 100; i++) {
  try { pid = adb('shell', 'pidof', 'tech.expoli.rustrss').trim(); } catch { pid = ''; }
  if (pid) break;
  await delay(100);
}
assert(pid, 'App process did not start');
adb('forward', 'tcp:9228', `localabstract:webview_devtools_remote_${pid}`);
let target;
for (let i = 0; i < 100; i++) {
  try {
    target = (await (await fetch('http://127.0.0.1:9228/json')).json())
      .find(row => row.type === 'page' && JSON.parse(row.description || '{}').attached);
    if (target) break;
  } catch { /* WebView is still starting. */ }
  await delay(100);
}
assert(target, 'App WebView did not start');
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((ready, reject) => { socket.onopen = ready; socket.onerror = reject; });
let id = 0;
const pending = new Map();
socket.onmessage = event => {
  const response = JSON.parse(event.data);
  const slot = pending.get(response.id);
  if (!slot) return;
  pending.delete(response.id);
  slot(response);
};
async function js(expression) {
  const next = ++id;
  const response = new Promise(resolveCall => pending.set(next, resolveCall));
  socket.send(JSON.stringify({ id: next, method: 'Runtime.evaluate', params: { expression, returnByValue: true, awaitPromise: true } }));
  const result = await response;
  assert(!result.result?.exceptionDetails, JSON.stringify(result.result?.exceptionDetails));
  return result.result.result.value;
}
try {
  for (let i = 0; i < 100; i++) {
    if (await js(`document.querySelectorAll('#tags li').length >= 2`)) break;
    await delay(100);
  }
  assert(await js(`document.querySelector('#tags').classList.contains('hidden')`), 'Tag section did not remain collapsed after app restart');
  const feed = (await js(`window.__TAURI__.core.invoke('list_feeds')`)).find(row => row.id === 2);
  const tags = await js(`window.__TAURI__.core.invoke('list_tags',{recentFirst:false})`);
  assert.equal(feed.custom_title, 'T2 edited feed');
  assert.equal(feed.refresh_interval_minutes, 30);
  assert.equal(tags.find(row => row.id === 2).pinned, false);
  await js(`document.querySelector('#tags-head').click()`);
  assert(await js(`!document.querySelector('#tags').classList.contains('hidden')`));
  const result = { serial, pid, collapsedAfterRestart: true, expandedAfterRestart: true,
    feedId: feed.id, feedName: feed.custom_title, interval: feed.refresh_interval_minutes,
    unpinnedTagId: 2, tagIds: tags.map(row => row.id), passed: true };
  writeFileSync(`${out}/restart-results.json`, JSON.stringify(result, null, 2) + '\n');
  console.log(JSON.stringify(result));
} finally {
  socket.close();
}
