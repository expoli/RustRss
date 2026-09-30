// Capture Android's accessibility tree on the task-owned AVD after T2 verification.
// ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9228/json node scripts/record-reading-experience-menu-talkback.mjs OUT
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const serial = process.env.ANDROID_SERIAL;
assert(serial, 'Select the task-owned emulator');
const out = resolve(process.argv[2]);
mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, ...args], { timeout: 30000, maxBuffer: 10 * 1024 * 1024 });
const pause = ms => new Promise(done => setTimeout(done, ms));
const service = 'com.google.android.marvin.talkback/com.google.android.marvin.talkback.TalkBackService';
adb('shell', 'settings', 'put', 'secure', 'enabled_accessibility_services', service);
adb('shell', 'settings', 'put', 'secure', 'accessibility_enabled', '1');
await pause(800);
const status = adb('shell', 'dumpsys', 'accessibility').toString();
const enabled = status.match(/Enabled services:\{\{[^\n]+/g)?.join('\n') || '';
assert(enabled.includes(service), 'TalkBack service did not enable');
writeFileSync(`${out}/talkback-service.txt`, `${enabled}\n`);

const target = (await (await fetch(process.env.CDP_URL || 'http://127.0.0.1:9228/json')).json())
  .find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target, 'Attached app WebView not found');
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((done, reject) => { socket.onopen = done; socket.onerror = reject; });
let id = 0;
const pending = new Map();
socket.onmessage = event => {
  const reply = JSON.parse(event.data);
  const slot = pending.get(reply.id);
  if (!slot) return;
  pending.delete(reply.id);
  slot(reply);
};
async function js(expression) {
  const key = ++id;
  const reply = new Promise(done => pending.set(key, done));
  socket.send(JSON.stringify({ id: key, method: 'Runtime.evaluate', params: { expression, returnByValue: true, awaitPromise: true } }));
  const result = await reply;
  assert(!result.result?.exceptionDetails, JSON.stringify(result.result?.exceptionDetails));
  return result.result.result.value;
}
async function tree(name, ready) {
  let xml = '';
  for (let attempt = 0; attempt < 10; attempt++) {
    adb('shell', 'uiautomator', 'dump', '/sdcard/t2-talkback.xml');
    xml = adb('exec-out', 'cat', '/sdcard/t2-talkback.xml').toString();
    if (ready(xml)) break;
    await pause(600);
  }
  assert(ready(xml), `Accessibility tree did not expose ${name}`);
  writeFileSync(`${out}/${name}.xml`, xml);
  writeFileSync(`${out}/${name}.png`, adb('exec-out', 'screencap', '-p'));
  return xml;
}
try {
  await js(`window.RustRssMenu?.close()`);
  await js(`document.querySelector('[data-mpage-btn="subscriptions"]').click()`);
  await js(`document.querySelector('#feeds li[data-feed-id="2"] .row-more').scrollIntoView({block:'center'})`);
  await js(`document.querySelector('#feeds li[data-feed-id="2"] .row-more').focus()`);
  await pause(800);
  const rows = await tree('talkback-rows', xml => xml.includes('更多操作：T2 edited feed'));
  assert(/<node[^>]+text="T2 edited feed[^>]+class="android\.widget\.Button"/.test(rows));
  assert(/<node[^>]+text="更多操作：T2 edited feed"[^>]+class="android\.widget\.Button"/.test(rows));
  await js(`document.querySelector('#feeds li[data-feed-id="2"] .row-more').click()`);
  await pause(400);
  await js(`(() => {[...document.querySelectorAll('.ctx-sheet-list button')].find(n=>n.textContent.includes('刷新间隔')).click()})()`);
  await pause(400);
  const interval = await tree('talkback-interval', xml => xml.includes('每 30 分钟') && xml.includes('checked="true"'));
  assert(/<node[^>]+text="每 30 分钟"[^>]+class="android\.view\.MenuItem"[^>]+checkable="true" checked="true"/.test(interval));
  const dom = await js(`(() => ({title:document.querySelector('#ctx-sheet-title')?.textContent,
    current:[...document.querySelectorAll('.ctx-sheet-list button')].filter(n=>n.getAttribute('aria-checked')==='true').map(n=>({text:n.textContent,role:n.getAttribute('role')})),
    inert:[...document.querySelectorAll('body > *')].filter(n=>n.id!=='ctx-menu').map(n=>({id:n.id,inert:n.inert}))}))()`);
  writeFileSync(`${out}/talkback-dom.txt`, JSON.stringify(dom, null, 2) + '\n');
  console.log(JSON.stringify({ serial, rowNames: true, selected: dom.current, enabled }));
} finally {
  socket.close();
}
