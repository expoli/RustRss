import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

assert.equal(process.env.ANDROID_SERIAL, 'emulator-5584');
const out = resolve(process.argv[2]); mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', process.env.ANDROID_SERIAL, ...args], { timeout: 30000 });
const wait = ms => new Promise(done => setTimeout(done, ms));
adb('shell', 'cmd', 'window', 'user-rotation', 'lock', '1');
let socket;
try {
  await wait(800);
  const system = adb('shell', 'dumpsys', 'input').toString().match(/Viewport INTERNAL: displayId=0,[^\n]+/)[0];
  assert(system.includes('orientation=1') && system.includes('logicalFrame=[0, 0, 2400, 1080]'), system);
  const target = (await (await fetch(process.env.CDP_URL)).json()).find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
  assert(target);
  socket = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((done, fail) => { socket.onopen = done; socket.onerror = fail; });
  let seq = 0; const pending = new Map();
  socket.onmessage = event => { const m = JSON.parse(event.data), p = pending.get(m.id); if (!p) return; pending.delete(m.id); m.error ? p.fail(m.error) : p.done(m.result); };
  async function js(expression) {
    const id = ++seq;
    const result = await new Promise((done, fail) => { pending.set(id, { done, fail }); socket.send(JSON.stringify({ id, method: 'Runtime.evaluate', params: { expression, returnByValue: true, awaitPromise: true } })); });
    assert(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
    return result.result.value;
  }
  await js(`document.querySelector('#m-nav [data-mpage-btn="settings"]').click();true`);
  await js(`document.querySelector('#tab-data').click();true`);
  const ui = await js(`({orientation:screen.orientation.type,width:innerWidth,height:innerHeight,android:document.body.dataset.android,mcp:!!document.querySelector('#tab-mcp').getClientRects().length,backup:!!document.querySelector('#act-backup-db').getClientRects().length,restore:!!document.querySelector('#act-restore-db').getClientRects().length,tray:!!document.querySelector('#set-close-action').getClientRects().length,hint:!!document.querySelector('#pane-data .m-only').getClientRects().length,opmlImport:!!document.querySelector('#act-import-opml').getClientRects().length,opmlExport:!!document.querySelector('#act-export-opml').getClientRects().length})`);
  assert(ui.orientation.startsWith('landscape') && ui.android === '1' && !ui.mcp && !ui.backup && !ui.restore && !ui.tray && ui.hint && ui.opmlImport && ui.opmlExport, JSON.stringify(ui));
  writeFileSync(`${out}/android-landscape.png`, adb('exec-out', 'screencap', '-p'));
  const report = { system, ui, apkSha256: process.env.APK_SHA256 };
  writeFileSync(`${out}/android-landscape-results.json`, JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify({ landscape: ui, out }));
} finally {
  socket?.close();
  adb('shell', 'cmd', 'window', 'user-rotation', 'lock', '0');
  adb('shell', 'cmd', 'window', 'user-rotation', 'free');
}
