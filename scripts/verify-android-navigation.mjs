// Isolated rebuilt debug APK; never run against a user's device/database.
// ANDROID_SERIAL=emulator-5580 node scripts/verify-android-navigation.mjs OUT [--baseline]
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
const serial = process.env.ANDROID_SERIAL;
assert(serial, 'Select the owned isolated emulator explicitly');
const out = resolve(process.argv[2]); mkdirSync(out, { recursive: true });
const baseline = process.argv.includes('--baseline');
const populated = process.argv.includes('--populated');
const adb = (...args) => execFileSync(process.env.ADB || '/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, ...args], { timeout: 30000 });
const wait = ms => new Promise(r => setTimeout(r, ms));
const targets = await (await fetch(process.env.CDP_URL || 'http://127.0.0.1:9227/json')).json();
// Changing Android's navigation mode recreates the Activity; DevTools may
// briefly retain its detached old WebView. Use the currently attached surface.
const target = targets.find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target, 'No attached application WebView');
const ws = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((r,j) => { ws.onopen=r; ws.onerror=j; });
let seq=0; const pending=new Map();
ws.onmessage=e=>{const m=JSON.parse(e.data),p=pending.get(m.id);if(p){clearTimeout(p.timer);pending.delete(m.id);m.error?p.reject(m.error):p.resolve(m.result);}};
const call=(method,params)=>new Promise((resolve,reject)=>{const id=++seq;const timer=setTimeout(()=>{pending.delete(id);reject(new Error(`DevTools timeout: ${method}`));},15000);pending.set(id,{resolve,reject,timer});ws.send(JSON.stringify({id,method,params}));});
async function js(expression) { const r=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});assert(!r.exceptionDetails,JSON.stringify(r.exceptionDetails));return r.result.value; }
const click=async selector=>{await js(`document.querySelector(${JSON.stringify(selector)}).click()`);await wait(400);};
const checks=[];
async function geometry(name) {
  const result=await js(`(() => {
    const rect=n=>{const r=n.getBoundingClientRect();return {x:r.x,y:r.y,right:r.right,bottom:r.bottom,width:r.width,height:r.height};};
    return {page:document.body.dataset.mpage,width:innerWidth,height:innerHeight,visual:{width:visualViewport.width,height:visualViewport.height},
      scrollWidth:document.documentElement.scrollWidth,nav:rect(document.querySelector('#m-nav')),
      items:[...document.querySelectorAll('#m-nav button')].map(n=>({name:n.dataset.mpageBtn,active:n.classList.contains('active'),button:rect(n),label:rect(n.querySelector('.m-label'))})),
      settingsOpen:!document.querySelector('#settings-overlay').classList.contains('hidden'),screen:document.querySelector('.settings-dialog').dataset.screen};
  })()`);
  // Android WebView may grow its layout viewport to fit overflowing content
  // while the physical visible viewport stays smaller. innerHeight alone lies.
  const complete=result.scrollWidth<=result.visual.width+1&&result.nav.bottom<=result.visual.height+1&&result.items.every(n=>n.label.bottom<=result.nav.bottom&&n.label.right<=result.visual.width+1&&n.button.height>=44);
  checks.push({name,complete,...result});
  writeFileSync(`${out}/${name}.png`,adb('exec-out','screencap','-p'));
  adb('shell','uiautomator','dump','/data/local/tmp/navigation-test.xml');
  writeFileSync(`${out}/${name}.xml`,adb('shell','cat','/data/local/tmp/navigation-test.xml'));
  if(!baseline) { assert(complete,`${name} clipped/overflow: ${JSON.stringify(result)}`);assert.equal(result.items.filter(n=>n.active).length,1);assert(result.items.find(n=>n.name===result.page)?.active); }
  return result;
}
async function native(selector) {
  const r=await js(`(() => {const r=document.querySelector(${JSON.stringify(selector)}).getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2,dpr:devicePixelRatio};})()`);
  // The attached surface metadata supplies the native origin. IME changes its
  // height, not this top inset; screenshot checkpoints retain native XML too.
  const top=JSON.parse(target.description).screenY;
  adb('shell','input','tap',String(Math.round(r.x*r.dpr)),String(Math.round(top+r.y*r.dpr)));await wait(400);
}
const back=async()=>{adb('shell','input','keyevent','KEYCODE_BACK');await wait(500);};
try {
  for(let i=0;i<100;i++){if(await js(`!!document.querySelector('#btn-settings').onclick`))break;await wait(100);}
  if(populated) await js(`window.__TAURI__.core.invoke('set_starred',{ids:[1,2],starred:true})`);
  await js(`I18N.setLocale('zh-CN');I18N.applyStaticI18n();document.documentElement.style.setProperty('--font-ui-size','18px')`);
  for(const page of ['articles','subscriptions','saved','settings']) {
    await click(`[data-mpage-btn="${page}"]`);
    await geometry(`primary-${page}`);
  }
  if(baseline) { assert(checks.some(c=>!c.complete)||checks.at(-1).page!=='settings','Baseline did not reproduce either regression'); }
  else {
    if(populated) {
      await native('[data-mpage-btn="articles"]');await click('[data-mview="all"]');
      assert((await js(`document.querySelectorAll('#entries li[data-id]').length`))>5);
      for(const offset of [180,'bottom']) {
      const id=await js(`(() => {window.__navRows=[...document.querySelectorAll('#entries li[data-id]')];const list=document.querySelector('#entries');list.scrollTop=${offset==='bottom'?'list.scrollHeight':offset};window.__navScroll=list.scrollTop;const box=list.getBoundingClientRect();return window.__navRows.find(n=>{const r=n.getBoundingClientRect();return r.top>=box.top&&r.bottom<=box.bottom;}).dataset.id;})()`);
      // Touch an actually visible row. Programmatically clicking an offscreen
      // row legitimately scrolls it into view before opening the reader.
      await native('#entries li[data-id="'+id+'"]');
      assert.equal(await js(`document.body.dataset.mpage`),'reader');
      assert.equal(await js(`getComputedStyle(document.querySelector('#m-nav')).display`),'none');
      writeFileSync(`${out}/reader.png`,adb('exec-out','screencap','-p'));
      await back();assert.equal(await js(`document.body.dataset.mpage`),'articles');
      assert(await js(`Math.abs(document.querySelector('#entries').scrollTop-window.__navScroll)<1 && window.__navRows.every(n=>n.isConnected)`));
      checks.push({name:'reader-back-preserves-list-scroll-and-rows-'+offset,passed:true});
      }
      await native('[data-mpage-btn="settings"]');
    }
    await native('#tab-reading');assert.equal(await js(`document.querySelector('.settings-dialog').dataset.screen`),'detail');
    await geometry('settings-reading');
    await js(`document.querySelector('#reading-editor [data-theme-field="typography.read_size"]').value='25';document.querySelector('#reading-editor [data-theme-field="typography.read_size"]').dispatchEvent(new Event('change',{bubbles:true}))`);
    await native('[data-mpage-btn="articles"]');
    assert.equal(await js(`document.querySelector('#settings-overlay').classList.contains('hidden')`),true);
    await native('[data-mpage-btn="settings"]');await native('#tab-reading');
    assert.notEqual(await js(`document.querySelector('#reading-editor [data-theme-field="typography.read_size"]').value`),'25');
    await back();assert.equal(await js(`document.querySelector('.settings-dialog').dataset.screen`),'home');
    await geometry('back-settings-home');await back();assert.equal(await js(`document.body.dataset.mpage`),'articles');
    await geometry('back-settings-origin');
    await native('[data-mpage-btn="subscriptions"]');await native('#btn-add');
    await wait(800);
    const ime=adb('shell','dumpsys','input_method').toString();assert(/mInputShown=true|mIsInputViewShown=true/.test(ime),'IME not shown');
    const input=await js(`(() => {const r=document.querySelector('#add-url').getBoundingClientRect();return {top:r.top,bottom:r.bottom,height:innerHeight};})()`);
    assert(input.top>=0&&input.bottom<=input.height);await geometry('subscriptions-ime');await back();
    for(const size of [14,18,24]) {
      await js(`document.documentElement.style.setProperty('--font-ui-size','${size}px')`);
      for(const page of ['articles','subscriptions','saved','settings']) {
        await native(`[data-mpage-btn="${page}"]`);
        await geometry(`font-${size}-${page}`);
      }
    }
    // Start the desktop check with Settings closed, matching a desktop launch.
    await native('[data-mpage-btn="articles"]');
    // The application media query goes dormant at desktop width. Preserve modal focus behavior.
    await call('Emulation.setDeviceMetricsOverride',{width:1240,height:820,deviceScaleFactor:1,mobile:false});await wait(400);
    await click('#btn-settings');
    assert.equal(await js(`document.querySelector('#settings-overlay').getAttribute('aria-modal')`),'true');
    assert.equal(await js(`document.querySelector('#settings-overlay').parentElement.tagName`),'BODY');
    await js(`document.querySelector('#settings-overlay').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}))`);
    assert.equal(await js(`document.querySelector('#settings-overlay').classList.contains('hidden')`),true);
    checks.push({name:'desktop-modal-media-regression',passed:true,limit:'Android WebView at desktop width; native Linux verified separately'});
  }
  writeFileSync(`${out}/results.json`,JSON.stringify({serial,baseline,checks},null,2)+'\n');
  console.log(JSON.stringify({out,checks:checks.length,baseline}));
} catch(error) {
  writeFileSync(`${out}/results.json`,JSON.stringify({serial,baseline,checks,error:String(error.stack)},null,2)+'\n');
  throw error;
} finally { await call('Emulation.clearDeviceMetricsOverride',{});ws.close(); }
