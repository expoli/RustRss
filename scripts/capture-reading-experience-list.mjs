// Capture identical Android list scenes before and after T3 from a reset fixture.
// ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9229/json node scripts/capture-reading-experience-list.mjs OUT PHASE
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const serial = process.env.ANDROID_SERIAL;
assert(serial, 'Select the task-owned AVD');
const out = resolve(process.argv[2]);
const phase = process.argv[3];
assert(['before', 'after'].includes(phase));
mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, ...args], { timeout: 30000 });
const pause = ms => new Promise(done => setTimeout(done, ms));
const target = (await (await fetch(process.env.CDP_URL || 'http://127.0.0.1:9229/json')).json())
  .find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target, 'Attached app WebView not found');
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((done, reject) => { ws.onopen = done; ws.onerror = reject; });
let id = 0;
const pending = new Map();
ws.onmessage = event => {
  const message = JSON.parse(event.data);
  const slot = pending.get(message.id);
  if (!slot) return;
  clearTimeout(slot.timer);
  pending.delete(message.id);
  message.error ? slot.reject(message.error) : slot.resolve(message.result);
};
const call = (method, params = {}) => new Promise((done, reject) => {
  const next = ++id;
  const timer = setTimeout(() => { pending.delete(next); reject(new Error(`DevTools timeout: ${method}`)); }, 15000);
  pending.set(next, { resolve: done, reject, timer });
  ws.send(JSON.stringify({ id: next, method, params }));
});
async function js(expression) {
  const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  assert(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
  return result.result.value;
}
async function until(expression) {
  for (let i = 0; i < 100; i++) {
    try { if (await js(expression)) return; } catch { /* page is reloading */ }
    await pause(100);
  }
  throw new Error(`Timed out: ${expression}`);
}

const results = { serial, phase, fixture: 'reading_experience_fixture 30, sha256 f4674f9df91a1c1fb1927087d93b0052169da0ecc298b782f8b98ff300aa1cba', scenes: [] };
try {
  await call('Page.enable');
  await until(`!!document.querySelector('#entries li[data-id]')`);
  for (const locale of ['zh-CN', 'en']) {
    for (const theme of ['light', 'dark']) {
      await js(`window.__TAURI__.core.invoke('set_ui_locale',{locale:${JSON.stringify(locale)}})`);
      await js(`window.__TAURI__.core.invoke('set_ui_theme',{theme:${JSON.stringify(theme)}})`);
      await call('Page.reload', { ignoreCache: true });
      await until(`document.documentElement.lang===${JSON.stringify(locale)} && document.documentElement.dataset.theme===${JSON.stringify(theme)} && !!document.querySelector('#entries li[data-id]')`);
      await pause(450);
      const metrics = await js(`(() => { const entry=document.querySelector('#entries li[data-id]'), title=entry?.querySelector('.title'), rect=n=>{const r=n.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,bottom:r.bottom}};
        const v=visualViewport; return {locale:document.documentElement.lang,theme:document.documentElement.dataset.theme,page:document.body.dataset.mpage,
          viewport:{width:v.width,height:v.height,offsetTop:v.offsetTop,innerHeight},
          firstId:entry?.dataset.id,firstTitle:title?.textContent,firstTitleRect:rect(title),
          firstTitleRatio:(rect(title).y-v.offsetTop)/v.height,
          listHead:rect(document.querySelector('.list-head')),toolbar:rect(document.querySelector('.toolbar')),
          listTitle:document.querySelector('#list-title')?.textContent,
          count:document.querySelector('#list-count')?.textContent,
          rowIds:[...document.querySelectorAll('#entries li[data-id]')].slice(0,8).map(n=>n.dataset.id),
          density:document.documentElement.dataset.density,summaryLines:document.documentElement.dataset.summaryLines,
          thumbnails:document.documentElement.dataset.thumbnails,
          css:{font:getComputedStyle(document.body).fontSize,background:getComputedStyle(document.body).backgroundColor}}
      })()`);
      const name = `${phase}-${locale}-${theme}`;
      writeFileSync(`${out}/${name}.png`, adb('exec-out', 'screencap', '-p'));
      results.scenes.push({ name, ...metrics });
    }
  }
  writeFileSync(`${out}/${phase}-scenes.json`, JSON.stringify(results, null, 2) + '\n');
  console.log(JSON.stringify({ phase, scenes: results.scenes.map(s => ({ name: s.name, viewport: s.viewport, firstTitleRatio: s.firstTitleRatio })) }));
} finally {
  ws.close();
}
