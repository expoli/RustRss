// Read the live Android WebView geometry after an AVD navigation, rotation, or font change.
// ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9229/json node scripts/verify-reading-experience-viewport.mjs OUT
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const serial = process.env.ANDROID_SERIAL;
assert(serial);
const out = resolve(process.argv[2]);
mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, ...args], { timeout: 30000 });
const targets = await (await fetch(process.env.CDP_URL || 'http://127.0.0.1:9229/json')).json();
const target = targets.find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target);
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((done, reject) => { ws.onopen = done; ws.onerror = reject; });
const result = await new Promise((done, reject) => {
  ws.onmessage = event => {
    const message = JSON.parse(event.data);
    message.error || message.result?.exceptionDetails ? reject(message.error || message.result.exceptionDetails) : done(message.result.result.value);
  };
  ws.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { returnByValue: true, expression: `(() => {
    const rect = node => { const r=node.getBoundingClientRect(); return {x:r.x,y:r.y,width:r.width,height:r.height,bottom:r.bottom}; };
    const nav = [...document.querySelectorAll('#m-nav button')].filter(n=>getComputedStyle(n).display!=='none').map(n=>({label:n.getAttribute('aria-label')||n.textContent,rect:rect(n)}));
    return {viewport:{width:visualViewport.width,height:visualViewport.height},page:document.body.dataset.mpage,
      nav,search:rect(document.querySelector('#btn-search-toggle')),firstTitle:rect(document.querySelector('#entries li[data-id] .title')),
      horizontalOverflow:document.documentElement.scrollWidth>visualViewport.width+1,bodyFont:getComputedStyle(document.body).fontSize}; })()` } }));
});
ws.close();
writeFileSync(`${out}/results.json`, JSON.stringify({serial,...result}, null, 2) + '\n');
writeFileSync(`${out}/screen.png`, adb('exec-out','screencap','-p'));
assert.equal(result.nav.length, 4);
assert(!result.horizontalOverflow);
assert(result.nav.every(item => item.rect.width >= 44 && item.rect.height >= 44 && item.rect.x >= 0 && item.rect.x + item.rect.width <= result.viewport.width + 1));
assert(result.nav.every(item => item.rect.y >= 0 && item.rect.bottom <= result.viewport.height + 1));
assert(result.firstTitle.y < result.viewport.height);
console.log(JSON.stringify({viewport:result.viewport,nav:result.nav.length,overflow:result.horizontalOverflow}));
