// Pixel Tablet hardware-profile AVD only. No user emulator or application data.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const serial = process.env.ANDROID_SERIAL;
assert.equal(serial, 'emulator-5586');
const out = resolve(process.argv[2]); mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, ...args], { timeout: 30000 }).toString().trim();
assert.equal(adb('shell', 'getprop', 'ro.boot.qemu.avd_name'), 'RustRssT6Tablet');
const profile = readFileSync('/home/tcy/.android/avd/RustRssT6Tablet.avd/config.ini', 'utf8');
assert.match(profile, /hw\.device\.name = pixel_tablet/);
const sleep = ms => new Promise(ok => setTimeout(ok, ms));
const checks = [];

async function measure(width, height) {
  adb('shell', 'wm', 'size', `${width * 2}x${height * 2}`);
  adb('shell', 'am', 'force-stop', 'tech.expoli.rustrss');
  adb('shell', 'am', 'start', '-n', 'tech.expoli.rustrss/.MainActivity');
  await sleep(4000);
  const pid = adb('shell', 'pidof', 'tech.expoli.rustrss').split(' ')[0];
  assert(pid, 'Tablet app process missing');
  adb('forward', 'tcp:9236', `localabstract:webview_devtools_remote_${pid}`);
  let page;
  for (let i = 0; i < 30; i++) {
    const pages = await (await fetch('http://127.0.0.1:9236/json')).json();
    page = pages.find(x => x.type === 'page' && JSON.parse(x.description || '{}').attached);
    if (page) break;
    await sleep(200);
  }
  assert(page, 'Tablet WebView missing');
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((ok, fail) => { ws.onopen = ok; ws.onerror = fail; });
  const result = await new Promise((ok, fail) => {
    const timer = setTimeout(() => fail(Error('CDP timeout')), 15000);
    ws.onmessage = event => {
      const value = JSON.parse(event.data);
      if (value.id !== 1) return;
      clearTimeout(timer);
      if (value.result?.exceptionDetails) fail(Error(JSON.stringify(value.result.exceptionDetails)));
      else ok(value.result.result.value);
    };
    ws.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { returnByValue: true,
      expression: `(() => {
        const el = s => document.querySelector(s);
        const box = s => { const n=el(s); if (!n) return null; const r=n.getBoundingClientRect(); return {x:r.x,y:r.y,w:r.width,h:r.height,display:getComputedStyle(n).display}; };
        return {css:[innerWidth,innerHeight],dpr:devicePixelRatio,coarse:matchMedia('(pointer:coarse)').matches,
          mobile:matchMedia('(max-width:960px) and (pointer:coarse)').matches, page:document.body.dataset.mpage,
          android:document.body.dataset.android,scrollWidth:document.documentElement.scrollWidth,
          listCopy:document.querySelector('#entries li')?.textContent,
          nav:box('#m-nav'),navButtons:[...document.querySelectorAll('#m-nav button')].filter(n=>n.getClientRects().length).length,
          settingsButton:box('#btn-settings'),columns:[box('.sidebar'),box('.list'),box('.right-col')],
          windowButtons:['#btn-win-min','#btn-win-max','#btn-win-close'].map(box),
          mcp:box('#tab-mcp'),backup:box('#act-backup-db'),opmlHint:box('#pane-data .m-only')};
      })()` } }));
  });
  writeFileSync(`${out}/tablet-${width}.png`, execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, 'exec-out', 'screencap', '-p']));
  let androidSettings;
  if (width === 1024) {
    androidSettings = await new Promise((ok, fail) => {
      const timer = setTimeout(() => fail(Error('Settings CDP timeout')), 15000);
      ws.onmessage = event => {
        const value = JSON.parse(event.data);
        if (value.id !== 2) return;
        clearTimeout(timer);
        if (value.result?.exceptionDetails) fail(Error(JSON.stringify(value.result.exceptionDetails)));
        else ok(value.result.result.value);
      };
      ws.send(JSON.stringify({ id: 2, method: 'Runtime.evaluate', params: { returnByValue: true,
        expression: `(() => { document.querySelector('#btn-settings').click(); document.querySelector('#tab-data').click();
          const visible=s=>!!document.querySelector(s)?.getClientRects().length;
          return {mcp:visible('#tab-mcp'),backup:visible('#act-backup-db'),restore:visible('#act-restore-db'),opmlHint:visible('#pane-data .m-only'),opmlExport:visible('#act-export-opml')}; })()` } }));
    });
    assert.deepEqual(androidSettings, {mcp:false,backup:false,restore:false,opmlHint:true,opmlExport:true}, androidSettings);
  }
  ws.close();
  assert.equal(result.css[0], width, `CSS width mismatch for ${width}`);
  assert.equal(result.dpr, 2);
  assert.equal(result.coarse, true);
  assert.equal(result.android, '1');
  assert(result.scrollWidth <= width + 1, `horizontal overflow ${width}: ${result.scrollWidth}`);
  if (width === 768) assert(/Open Subscriptions|前往「订阅」/.test(result.listCopy), result.listCopy);
  if (width === 1024) assert(/Add feed above|上方「添加订阅」/.test(result.listCopy), result.listCopy);
  if (width <= 960) {
    assert(result.mobile && result.nav.display !== 'none' && result.navButtons === 4);
    assert.equal(result.settingsButton.display, 'none');
  } else {
    assert(!result.mobile && result.nav.display === 'none' && result.navButtons === 0);
    assert(result.settingsButton.display !== 'none');
    assert(result.columns.every(c => c.display !== 'none' && c.w > 0), `desktop-like tablet panes: ${JSON.stringify(result.columns)}`);
  }
  assert(result.windowButtons.every(b => b.w === 0 && b.h === 0), 'Android desktop window buttons visible');
  const item = {width,height,physical:adb('shell','wm','size'),density:adb('shell','wm','density'),...result,androidSettings};
  checks.push(item);
  process.stdout.write(JSON.stringify({width,css:result.css,mobile:result.mobile,columns:result.columns.map(c=>c.w),scrollWidth:result.scrollWidth})+'\n');
}

try {
  assert.match(adb('shell','wm','density'), /320/);
  for (const [w,h] of [[768,1024],[849,1024],[850,1024],[851,1024],[959,1024],[960,1024],[961,1024],[1024,768]]) await measure(w,h);
  writeFileSync(`${out}/tablet-results.json`,JSON.stringify({device:serial,avd:'RustRssT6Tablet',profile:'pixel_tablet',android:adb('shell','getprop','ro.build.version.release'),apkSha256:createHash('sha256').update(readFileSync('src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk')).digest('hex'),checks},null,2)+'\n');
} finally {
  adb('shell','wm','size','reset');
  adb('shell','wm','density','reset');
}
