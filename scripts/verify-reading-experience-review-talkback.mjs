// Native TalkBack keyboard shortcuts on the task-owned AVD. Requires the
// temporary t4-focus-logger AccessibilityService alongside TalkBack.
// ANDROID_SERIAL=emulator-5582 node scripts/verify-reading-experience-review-talkback.mjs OUT
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const serial = process.env.ANDROID_SERIAL;
assert.equal(serial, 'emulator-5582');
const out = resolve(process.argv[2]); mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, ...args], { timeout: 30000, maxBuffer: 12 * 1024 * 1024 });
const wait = ms => new Promise(done => setTimeout(done, ms));
const services = adb('shell', 'dumpsys', 'accessibility').toString();
assert(services.includes('Bound services:') && services.includes('label=TalkBack') && services.includes('label=T4 Focus Logger') && services.includes('TouchExplorer'));
const target = (await (await fetch(process.env.CDP_URL || 'http://127.0.0.1:9229/json')).json())
  .find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target);
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
  for (let i = 0; i < 100; i++) { if (await js(expression)) return; await wait(100); }
  throw new Error(`Timed out: ${expression}`);
}
function physicalKey(...codes) {
  const commands = [...codes.map(code => `sendevent /dev/input/event1 1 ${code} 1`),
    'sendevent /dev/input/event1 0 0 0',
    ...codes.toReversed().map(code => `sendevent /dev/input/event1 1 ${code} 0`),
    'sendevent /dev/input/event1 0 0 0'];
  adb('shell', commands.join('; '));
}
const talkBackNext = () => physicalKey(56, 106); // Alt + Right
const talkBackClick = () => physicalKey(56, 28); // Alt + Enter
const talkBackBack = () => physicalKey(56, 14); // Alt + Backspace/Delete
const focusFirst = () => physicalKey(56, 29, 105); // Alt + Ctrl + Left
const trace = () => adb('logcat', '-d', '-s', 'T4A11Y:I').toString();
const focusLines = () => trace().split('\n').filter(line => line.includes('TYPE_VIEW_ACCESSIBILITY_FOCUSED package=tech.expoli.rustrss'));
const focusId = line => line?.match(/ id=([^ ]+)/)?.[1] || null;
const entry = () => js(`window.__TAURI__.core.invoke('get_entry',{id:1})`);
try {
  await until(`!!document.querySelector('#entries li[data-id="1"] .title')`);
  // CDP only prepares the synthetic article. All acceptance actions below are
  // physical keyboard events delivered to TalkBack through the emulator input device.
  await js(`document.querySelector('#entries li[data-id="1"] .title').click()`);
  await until(`document.body.dataset.mpage==='reader'`);
  assert.equal((await entry()).read, true);
  assert.equal(await js(`document.querySelector('main > .list').inert`), true);
  adb('logcat', '-c');
  focusFirst();
  for (let i = 0; i < 25 && focusId(focusLines().at(-1)) !== 'act-more'; i++) {
    talkBackNext(); await wait(120);
  }
  const toolbarFocus = focusLines();
  const ids = toolbarFocus.map(focusId);
  const expected = ['m-reader-back', 'act-aa', 'act-star', 'act-later', 'act-more'];
  let cursor = -1;
  for (const id of expected) {
    cursor = ids.indexOf(id, cursor + 1);
    assert(cursor >= 0, `TalkBack traversal missing ${id}: ${ids.join(', ')}`);
  }
  assert(!ids.some(id => ['list-count', 'btn-list-bulk', 'btn-list-sort', 'entries'].includes(id)), 'Covered list leaked into reader traversal');
  talkBackClick();
  await until(`!!document.querySelector('.ctx-sheet-list')`);
  await wait(250);
  const menuFocus = focusLines().at(-1);
  assert(menuFocus.includes('text=标为未读') && menuFocus.includes('a11yFocus=true'), menuFocus);
  const items = await js(`[...document.querySelectorAll('.ctx-sheet-list button')].map(n=>({label:n.textContent,role:n.getAttribute('role'),checked:n.getAttribute('aria-checked')}))`);
  // uiautomator dump reconnects accessibility services and clears TalkBack's
  // focused node, so retain the event stream and a system screenshot instead.
  writeFileSync(`${out}/review-talkback-more.png`, adb('exec-out', 'screencap', '-p'));
  talkBackClick();
  await until(`window.__TAURI__.core.invoke('get_entry',{id:1}).then(e=>!e.read)`);
  await until(`!document.querySelector('.ctx-sheet-list')`);
  await wait(250);
  assert.equal(await js(`document.activeElement?.id`), 'act-more');
  assert.equal(focusId(focusLines().at(-1)), 'act-more');
  talkBackClick();
  await until(`!!document.querySelector('.ctx-sheet-list')`);
  talkBackBack();
  await until(`!document.querySelector('.ctx-sheet-list')`);
  await wait(250);
  assert.equal((await entry()).read, false, 'TalkBack Back must not write');
  assert.equal(await js(`document.activeElement?.id`), 'act-more');
  assert.equal(focusId(focusLines().at(-1)), 'act-more');
  writeFileSync(`${out}/review-talkback-closed.png`, adb('exec-out', 'screencap', '-p'));
  const log = trace();
  writeFileSync(`${out}/review-talkback-events.log`, log);
  const result = { serial, source: 'physical /dev/input/event1 Alt+Right, Alt+Enter and Alt+Backspace with TalkBack bound',
    services: ['TalkBack', 'T4 Focus Logger'], listInert: true, toolbarFocus: ids,
    menuFirstFocus: menuFocus, menuItems: items, readBefore: true, readAfterAction: false,
    closeFocus: focusId(focusLines().at(-1)), backCancelledWrite: true,
    speechCapture: 'not available on emulator', passed: true };
  writeFileSync(`${out}/review-talkback-results.json`, JSON.stringify(result, null, 2) + '\n');
  console.log(JSON.stringify({ toolbar: expected, action: 'mark unread', closeFocus: result.closeFocus }));
} finally { socket.close(); }
