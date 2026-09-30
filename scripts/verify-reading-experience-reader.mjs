// Native tap acceptance for the task-owned Android fixture. CDP reads state and sets only fixture inputs.
// ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9229/json node scripts/verify-reading-experience-reader.mjs OUT
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const serial = process.env.ANDROID_SERIAL;
assert.equal(serial, 'emulator-5582', 'T4 only drives its owned emulator');
const out = resolve(process.argv[2]);
mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, ...args], { timeout: 30000 });
const wait = ms => new Promise(done => setTimeout(done, ms));
const target = (await (await fetch(process.env.CDP_URL || 'http://127.0.0.1:9229/json')).json())
  .find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target, 'Attached Android WebView missing');
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((done, fail) => { socket.onopen = done; socket.onerror = fail; });
let seq = 0;
const pending = new Map();
socket.onmessage = event => {
  const m = JSON.parse(event.data), slot = pending.get(m.id);
  if (!slot) return;
  clearTimeout(slot.timer); pending.delete(m.id);
  m.error ? slot.fail(m.error) : slot.done(m.result);
};
const call = (method, params = {}) => new Promise((done, fail) => {
  const id = ++seq, timer = setTimeout(() => { pending.delete(id); fail(new Error(`CDP timeout ${method}`)); }, 15000);
  pending.set(id, { done, fail, timer });
  socket.send(JSON.stringify({ id, method, params }));
});
async function js(expression) {
  const r = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  assert(!r.exceptionDetails, JSON.stringify(r.exceptionDetails));
  return r.result.value;
}
async function until(expression) {
  for (let i = 0; i < 100; i++) {
    try { if (await js(expression)) return; } catch { /* navigation */ }
    await wait(100);
  }
  throw new Error(`Timed out: ${expression}`);
}
async function native(selector) {
  const p = await js(`(() => {const n=document.querySelector(${JSON.stringify(selector)}); if(!n)return null;
    n.scrollIntoView({block:'nearest'});const r=n.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;
    return {x,y,w:r.width,h:r.height,dpr:devicePixelRatio,visible:r.width>0&&r.height>0&&y>=0&&y<visualViewport.height&&document.elementFromPoint(x,y)?.closest(${JSON.stringify(selector)})===n};})()`);
  assert(p?.visible, `Not tappable ${selector}: ${JSON.stringify(p)}`);
  const y = JSON.parse(target.description).screenY + p.y * p.dpr;
  if (process.env.T4_TRACE) console.error('TAP', selector, Math.round(p.x * p.dpr), Math.round(y), p);
  adb('shell', 'input', 'tap', String(Math.round(p.x * p.dpr)), String(Math.round(y)));
  await wait(280);
}
async function menu(label) {
  const found = await js(`(() => {const b=[...document.querySelectorAll('.ctx-sheet-list button')].find(b=>b.textContent.includes(${JSON.stringify(label)}));if(!b)return false;b.dataset.t4Target='true';return true})()`);
  assert(found, `Missing menu action: ${label}`);
  await native('.ctx-sheet-list button[data-t4-target="true"]');
  await wait(800); // Let Android WebView consume the history entry before CDP polling.
}
async function more() { await native('#act-more'); await until(`!!document.querySelector('.ctx-sheet-list')`); await wait(450); }
function back() { adb('shell', 'input', 'keyevent', 'KEYCODE_BACK'); }
function shot(name) { writeFileSync(`${out}/${name}.png`, adb('exec-out', 'screencap', '-p')); }
const checks = [];
function record(name, data) { checks.push({ name, ...data }); }
const entry = id => js(`window.__TAURI__.core.invoke('get_entry',{id:${id}})`);
const count = () => Number(readFileSync('/tmp/rustrss-t4-ai-count.txt', 'utf8'));
const pageCount = () => Number(readFileSync('/tmp/rustrss-t4-page-count.txt', 'utf8'));
writeFileSync('/tmp/rustrss-t4-page-mode.txt', 'fail');
try {
  await until(`!!document.querySelector('#entries li[data-id="1"]')`);
  await js(`window.__T4_ROW=document.querySelector('#entries li[data-id="1"]')`);
  await native('#entries li[data-id="1"] .title');
  await until(`document.body.dataset.mpage==='reader'`);
  const layout = await js(`(() => {const r=s=>{const n=document.querySelector(s),a=n.getBoundingClientRect();return {x:a.x,y:a.y,width:a.width,height:a.height,bottom:a.bottom}};
    return {viewport:{width:visualViewport.width,height:visualViewport.height},first:r('.article p'),title:r('.reader-head h1'),meta:r('.reader-head .meta'),actions:r('.reader-actions'),back:r('#m-reader-back'),
      buttons:[...document.querySelectorAll('.reader-actions button')].map(b=>({id:b.id,width:b.getBoundingClientRect().width,height:b.getBoundingClientRect().height,aria:b.getAttribute('aria-label')})),pageScrollWidth:document.documentElement.scrollWidth};})()`);
  assert.equal(layout.viewport.width, 360);
  assert(layout.first.y / layout.viewport.height <= .4, JSON.stringify(layout));
  assert(layout.buttons.every(b => b.height >= 48 && b.width >= 48));
  assert(layout.back.height >= 48 && layout.back.width >= 48);
  assert(layout.meta.height <= 31 && layout.pageScrollWidth <= 360);
  record('reader-geometry-360', { ...layout, firstRatio: layout.first.y / layout.viewport.height });
  await until(`!!document.querySelector('.reader-image-fallback')`);
  const longContent = await js(`(() => {const r=document.querySelector('#reader'),a=document.querySelector('.article');return {paragraphs:a.querySelectorAll('p').length,codeBlocks:a.querySelectorAll('pre').length,tables:a.querySelectorAll('table').length,offlineImageFallback:a.querySelector('.reader-image-fallback')?.textContent,readerOverflow:r.scrollWidth>r.clientWidth,articleOverflow:a.scrollWidth>a.clientWidth}})()`);
  assert(longContent.paragraphs >= 40 && longContent.codeBlocks && longContent.tables);
  assert.equal(longContent.offlineImageFallback, 'Fixture illustration');
  assert(!longContent.readerOverflow && !longContent.articleOverflow);
  record('long-mixed-content-offline-image', longContent);
  const initialRead = (await entry(1)).read;
  assert.equal(initialRead, true, 'A direct list tap marks the opened article read');
  const markReadOnNavigate = (await js(`window.__TAURI__.core.invoke('get_ui_settings')`)).mark_read_on_navigate;
  assert.equal(markReadOnNavigate, false, 'Fixture setting must be read from SQLite through the app');
  const readSync = await js(`({sameRow:window.__T4_ROW===document.querySelector('#entries li[data-id="1"]'),rowRead:window.__T4_ROW.classList.contains('read')})`);
  assert(readSync.sameRow && readSync.rowRead, 'Auto-read must patch the existing row');
  shot('01-reader');

  await js(`(() => {window.__T4_EVENTS=[];for(const n of ['rustrss-menu-close','rustrss-menu-action','popstate'])window.addEventListener(n,()=>window.__T4_EVENTS.push(n));return true})()`);
  const beforeMenu = { ai:count(), fulltext:pageCount() };
  await more();
  const opened = await js(`({rows:[...document.querySelectorAll('.ctx-sheet-list button')].map(n=>n.textContent),minHeight:Math.min(...[...document.querySelectorAll('.ctx-sheet-list button')].map(n=>n.getBoundingClientRect().height))})`);
  assert.deepEqual({ ai:count(), fulltext:pageCount() }, beforeMenu, 'Opening More must not send AI or fulltext');
  assert(opened.rows.some(x => x.includes('获取全文')) && opened.rows.some(x => x.includes('AI 摘要')));
  assert(opened.rows.some(x => x.includes('标为未读')), 'More must reflect the stored auto-read state');
  assert(opened.minHeight >= 48);
  record('more-opens-with-zero-network-actions', { ...opened, beforeMenu, afterMenu:{ ai:count(), fulltext:pageCount() }, autoReadRowIdentity:readSync.sameRow, autoReadRowClass:readSync.rowRead });
  shot('02-more');
  back();
  await until(`!document.querySelector('.ctx-sheet-list')`);
  const menuBack = await js(`({focus:document.activeElement?.id,page:document.body.dataset.mpage})`);
  assert.equal(menuBack.focus, 'act-more');
  assert.equal(menuBack.page, 'reader');
  assert.deepEqual({ ai:count(), fulltext:pageCount() }, beforeMenu);
  record('more-native-back-focus-zero-actions', menuBack);
  await more();
  await menu('标为未读');
  if (process.env.T4_TRACE) console.error('EVENTS', await js(`window.__T4_EVENTS`), 'READ', (await entry(1)).read);
  await until(`window.__TAURI__.core.invoke('get_entry',{id:1}).then(r=>!r.read)`);
  await native('#act-star');
  await until(`window.__TAURI__.core.invoke('get_entry',{id:1}).then(r=>r.starred)`);
  await native('#act-later');
  await until(`window.__TAURI__.core.invoke('get_entry',{id:1}).then(r=>r.read_later)`);
  record('read-star-later-store', { markReadOnNavigate, directListTapRead:initialRead, afterMenuUnread:(await entry(1)).read,starred:(await entry(1)).starred,later:(await entry(1)).read_later });

  await more(); await menu('管理标签');
  await until(`!document.querySelector('#tag-picker-overlay').classList.contains('hidden')`);
  await js(`(() => {const n=document.querySelector('#tag-picker-input');n.value='T4Tag';n.dispatchEvent(new Event('input',{bubbles:true}));return true})()`);
  await native('#tag-picker-list li.create');
  await until(`window.__TAURI__.core.invoke('get_entry',{id:1}).then(r=>r.tags.some(t=>t.name==='T4Tag'))`);
  await js(`(() => {const n=document.querySelector('#tag-picker-input');n.value='T4';n.dispatchEvent(new Event('input',{bubbles:true}));return true})()`);
  assert(await js(`!!document.querySelector('#tag-picker-list li[data-tag-id]')`));
  await native('#tag-picker-list li[data-tag-id]');
  await until(`window.__TAURI__.core.invoke('get_entry',{id:1}).then(r=>r.tags.length===0)`);
  await native('#tag-picker-list li[data-tag-id]');
  await until(`window.__TAURI__.core.invoke('get_entry',{id:1}).then(r=>r.tags.length===1)`);
  const chip = await js(`(() => {const n=document.querySelector('#reader-tags .tag-chip');return {role:n?.getAttribute('role'),tabIndex:n?.tabIndex,height:n?.getBoundingClientRect().height}})()`);
  assert(chip.role==='button' && chip.tabIndex===0 && chip.height>=48);
  record('tag-search-create-attach-remove', { attached:(await entry(1)).tags.map(t=>t.name),chip });
  await native('#tag-picker-close');

  await more(); await menu('复制链接');
  await until(`window.__TAURI__.core.invoke('clip_read').then(x=>x==='http://10.0.2.2:18080/missing')`);
  const copied = await js(`window.__TAURI__.core.invoke('clip_read')`);
  assert.equal(copied, 'http://10.0.2.2:18080/missing');
  record('copy-link-clipboard', { matchesFixture: true });
  await more(); await menu('浏览器打开');
  await wait(650);
  const browserUi = adb('shell', 'dumpsys', 'window').toString();
  assert(/chrome|browser|ResolverActivity/i.test(browserUi), 'System browser activity missing');
  shot('03-external-browser');
  back(); await wait(450);
  const browserReturned = await js(`document.body.dataset.mpage==='reader'`);
  assert(browserReturned, 'Android Back after browser must restore the reader');
  record('external-browser-open-back', { opened:true, returnedToReader:browserReturned });
  await more(); await menu('分享');
  await wait(800);
  const shareUi = adb('shell', 'dumpsys', 'window').toString();
  assert(/ChooserActivity|ResolverActivity|IntentResolver|android:chooser/.test(shareUi), 'Native share sheet missing');
  shot('04-native-share');
  back(); await wait(450);
  const shareReturned = await js(`document.body.dataset.mpage==='reader'`);
  assert(shareReturned, 'Android Back after native share must restore the reader');
  record('share-native-open-cancel', { sheet: true, returnedToReader:shareReturned });

  const beforeFetch = pageCount();
  await more(); await menu('获取全文');
  for (let i = 0; i < 40 && pageCount() === beforeFetch; i++) await wait(100);
  assert(pageCount() > beforeFetch, 'Fulltext request did not reach synthetic endpoint');
  await wait(600);
  assert((await entry(1)).needs_fulltext, 'Failed full text must remain retriable');
  record('fulltext-failure-retry-available', { needsFulltext:(await entry(1)).needs_fulltext, endpointRequests:pageCount() });
  writeFileSync('/tmp/rustrss-t4-page-mode.txt', 'recover');
  await more(); await menu('获取全文');
  await until(`window.__TAURI__.core.invoke('get_entry',{id:1}).then(r=>!r.needs_fulltext)`);
  assert(pageCount()===beforeFetch+2, 'Exactly one request for each explicit full text action');
  record('fulltext-retry-success-store', { needsFulltext:(await entry(1)).needs_fulltext, endpointRequests:pageCount() });

  await native('#m-reader-back');
  await until(`document.body.dataset.mpage==='articles'`);
  await native('#m-seg-articles [data-mview="all"]');
  await until(`!!document.querySelector('#entries li[data-id="2"]')`);
  await native('#entries li[data-id="2"] .title');
  await until(`document.body.dataset.mpage==='reader'`);
  await more(); await menu('获取全文');
  await until(`window.__TAURI__.core.invoke('get_entry',{id:2}).then(r=>!r.needs_fulltext)`);
  await until(`document.querySelector('.article')?.textContent.includes('Offline fixture paragraph')`);
  const shortTitle = await js(`(() => {const n=document.querySelector('.reader-head h1'),a=n.getBoundingClientRect(),p=document.querySelector('.article p').getBoundingClientRect();return {titleLines:a.height/parseFloat(getComputedStyle(n).lineHeight),firstRatio:p.top/visualViewport.height}})()`);
  assert(shortTitle.titleLines <= 2.05 && shortTitle.firstRatio <= .4);
  record('short-title-first-paragraph', shortTitle);
  record('fulltext-success-store', { needsFulltext:(await entry(2)).needs_fulltext, hasSyntheticText:true });
  shot('05-fulltext');

  await js(`window.__TAURI__.core.invoke('save_ai_settings',{provider:'ollama',model:'synthetic-t4',baseUrl:'http://10.0.2.2:18080',translateTarget:'en',apiKey:null,maxOutputTokens:256})`);
  await js(`window.__TAURI__.core.invoke('set_ai_confirm_before_send',{enabled:true})`);
  // Reload state.ai through the app's ordinary startup path; the same article is reopened by native tap.
  await call('Page.reload', { ignoreCache:true });
  await until(`!!document.querySelector('#m-seg-articles [data-mview="all"]')`);
  await native('#m-seg-articles [data-mview="all"]');
  await until(`!!document.querySelector('#entries li[data-id="2"]')`);
  await native('#entries li[data-id="2"] .title');
  await until(`document.body.dataset.mpage==='reader'`);
  const beforeAi = count();
  await more(); await menu('AI 摘要');
  await until(`!document.querySelector('#ai-confirm-overlay').classList.contains('hidden')`);
  const preview = await js(`({url:document.querySelector('#ai-confirm-url').textContent,headers:document.querySelector('#ai-confirm-headers').textContent,bodyChars:document.querySelector('#ai-confirm-body').textContent.length,summary:document.querySelector('#ai-confirm-summary').textContent})`);
  assert.equal(preview.url, 'http://10.0.2.2:18080/api/generate');
  assert(preview.bodyChars > 50 && !/Bearer|sk-/.test(preview.headers));
  assert.equal(count(), beforeAi, 'Preview sent a network request');
  record('ai-confirm-synthetic-preview', { endpoint:preview.url, headersRedacted:true, bodyChars:preview.bodyChars, requestCountBeforeSend:beforeAi });
  shot('06-ai-confirm-synthetic');
  await native('#ai-confirm-cancel');
  await wait(250);
  assert.equal(count(), beforeAi);
  record('ai-cancel-zero-send', { requestCount:count() });
  await more(); await menu('AI 摘要');
  await until(`!document.querySelector('#ai-confirm-overlay').classList.contains('hidden')`);
  await native('#ai-confirm-send');
  await until(`document.querySelector('#ai-panel-body')?.textContent.includes('Synthetic summary result')`);
  assert.equal(count(), beforeAi + 1);
  record('ai-summary-result', { kind:await js(`document.querySelector('#ai-panel-title').textContent`),meta:await js(`document.querySelector('#ai-panel-meta').textContent`),requestCount:count() });
  shot('07-ai-result');
  await native('#ai-regenerate');
  await until(`!document.querySelector('#ai-confirm-overlay').classList.contains('hidden')`);
  await native('#ai-confirm-send');
  await until(`document.querySelector('#ai-panel-body')?.textContent.includes('result #${beforeAi + 2}')`);
  record('ai-regenerate-replaced', { requestCount:count() });
  await native('#ai-close');
  assert(await js(`document.querySelector('#ai-panel').classList.contains('hidden')`));
  const articleAfterAiClose = await js(`document.querySelector('.article')?.textContent.includes('Offline fixture paragraph')`);
  assert(articleAfterAiClose);
  assert.equal(count(), beforeAi + 2);
  record('ai-close-keeps-article', { panelHidden:true, articleRetained:articleAfterAiClose, requestCount:count() });
  await more(); await menu('AI 翻译');
  await until(`!document.querySelector('#ai-confirm-overlay').classList.contains('hidden')`);
  await native('#ai-confirm-send');
  await until(`document.querySelector('#ai-panel-body')?.textContent.includes('Synthetic translate result')`);
  record('ai-translation-result', { kind:await js(`document.querySelector('#ai-panel-title').textContent`),requestCount:count() });
  writeFileSync('/tmp/rustrss-t4-ai-mode.txt', 'fail');
  await native('#ai-regenerate');
  await until(`!document.querySelector('#ai-confirm-overlay').classList.contains('hidden')`);
  await native('#ai-confirm-send');
  await until(`document.querySelector('#ai-panel-meta')?.textContent.includes('失败')`);
  record('ai-failure-retry', { failed:true, requestCount:count() });
  writeFileSync('/tmp/rustrss-t4-ai-mode.txt', 'ok');
  await native('#ai-regenerate');
  await until(`!document.querySelector('#ai-confirm-overlay').classList.contains('hidden')`);
  await native('#ai-confirm-dont-ask');
  await native('#ai-confirm-send');
  await until(`document.querySelector('#ai-panel-body')?.textContent.includes('Synthetic translate result')`);
  assert.equal((await js(`window.__TAURI__.core.invoke('get_ai_settings')`)).confirm_before_send, false);
  const recoveredMeta = await js(`({kind:document.querySelector('#ai-panel-title')?.textContent,meta:document.querySelector('#ai-panel-meta')?.textContent})`);
  assert(recoveredMeta.kind.includes('翻译') && !recoveredMeta.meta.includes('失败'));
  assert.equal(count(), beforeAi + 5, 'Each explicit send must issue exactly one request');
  record('ai-retry-and-dont-ask', { recovered:true, confirmBeforeSend:false, requestCount:count(),...recoveredMeta });
  shot('08-ai-recovered');

  await js(`document.querySelector('#reader').scrollTop=900`);
  const anchor = () => js(`(() => {const r=document.querySelector('#reader'),y=r.getBoundingClientRect().top+8,p=[...r.querySelectorAll('.article > *')].find(n=>n.getBoundingClientRect().bottom>y);return {text:p?.textContent?.slice(0,35),top:p?.getBoundingClientRect().top,scrollTop:r.scrollTop}})()`);
  const beforeAa = await anchor();
  const fontBefore = await js(`parseFloat(getComputedStyle(document.querySelector('.article')).fontSize)`);
  const revisionBefore = await js(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.theme_snapshot.config.revision)`);
  await native('#act-aa');
  await until(`document.querySelector('#aa-dialog').open`);
  await js(`(() => {const n=document.querySelector('#aa-editor [data-theme-field="typography.read_size"]');n.value=String(Number(n.value)+1);n.dispatchEvent(new Event('change',{bubbles:true}));return true})()`);
  await native('#aa-editor .theme-editor-actions button:nth-child(2)');
  await until(`document.querySelector('#aa-editor .theme-preview-details')?.open`);
  assert.equal(await js(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.theme_snapshot.config.revision)`), revisionBefore);
  await native('#aa-editor .theme-editor-actions button:nth-child(3)');
  await native('#aa-close');
  const afterCancel = await anchor();
  assert.equal(afterCancel.text, beforeAa.text);
  assert(Math.abs(afterCancel.top-beforeAa.top)<3);
  await native('#act-aa');
  await until(`document.querySelector('#aa-dialog').open`);
  await js(`(() => {const n=document.querySelector('#aa-editor [data-theme-field="typography.read_size"]');n.value=String(Number(n.value)+1);n.dispatchEvent(new Event('change',{bubbles:true}));return true})()`);
  await native('#aa-editor .theme-editor-actions button:first-child');
  await until(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.theme_snapshot.config.revision>${revisionBefore})`);
  await native('#aa-close');
  const afterSave = await anchor();
  const fontAfter = await js(`parseFloat(getComputedStyle(document.querySelector('.article')).fontSize)`);
  const storedReadSize = await js(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.theme_snapshot.config.overrides.typography.read_size)`);
  assert(fontAfter>fontBefore && storedReadSize===fontAfter, 'Aa save must apply and persist its size');
  assert.equal(afterSave.text, beforeAa.text, 'Aa save must preserve the same paragraph anchor');
  assert(Math.abs(afterSave.top-beforeAa.top)<25, 'Aa save must preserve visible anchor position');
  record('aa-preview-cancel-save-anchor', { revisionBefore,revisionAfter:await js(`window.__TAURI__.core.invoke('get_ui_settings').then(s=>s.theme_snapshot.config.revision)`),fontBefore,fontAfter,storedReadSize,beforeAa,afterCancel,afterSave });
  shot('09-aa-saved-anchor');

  const listBefore = await js(`({view:document.querySelector('#views li.active')?.dataset.kind,scrollTop:document.querySelector('#entries').scrollTop,selected:document.querySelector('#entries li.active')?.dataset.id,count:document.querySelector('#list-count')?.textContent,search:document.querySelector('#search')?.value})`);
  await native('#m-reader-back');
  await until(`document.body.dataset.mpage==='articles'`);
  const listAfter = await js(`({view:document.querySelector('#views li.active')?.dataset.kind,scrollTop:document.querySelector('#entries').scrollTop,selected:document.querySelector('#entries li.active')?.dataset.id,count:document.querySelector('#list-count')?.textContent,search:document.querySelector('#search')?.value})`);
  assert.deepEqual(listAfter,listBefore);
  record('reader-back-list-state', { before:listBefore,after:listAfter });
  await js(`document.querySelector('#entries').scrollTop=500`);
  const allScroll = await js(`document.querySelector('#entries').scrollTop`);
  await native('#btn-search-toggle');
  await native('#search');
  await until(`visualViewport.height<600`);
  const imeHeight = await js(`visualViewport.height`);
  adb('shell','input','text','English');
  await until(`document.body.dataset.listKind==='search' && !!document.querySelector('#entries li[data-id]')`);
  back(); await wait(350); // close the IME
  const searchBefore = await js(`({query:document.querySelector('#search').value,kind:document.body.dataset.listKind,count:document.querySelector('#list-count').textContent,rowId:document.querySelector('#entries li[data-id]')?.dataset.id})`);
  assert.equal(searchBefore.query,'English');
  await native('#entries li[data-id] .title');
  await until(`document.body.dataset.mpage==='reader'`);
  back();
  await until(`document.body.dataset.mpage==='articles'`);
  const searchAfter = await js(`({query:document.querySelector('#search').value,kind:document.body.dataset.listKind,count:document.querySelector('#list-count').textContent,rowId:document.querySelector('#entries li[data-id]')?.dataset.id})`);
  assert.deepEqual(searchAfter,searchBefore);
  record('search-reader-android-back-ime', { before:searchBefore,after:searchAfter,imeHeight });
  await native('#btn-search-cancel');
  await until(`document.querySelector('#views li.active')?.dataset.kind==='all'`);
  const restoredScroll = await js(`document.querySelector('#entries').scrollTop`);
  assert(Math.abs(restoredScroll-allScroll)<3);
  record('search-cancel-all-scroll-anchor', { before:allScroll,after:restoredScroll });
  await native('#entries li[data-id="1"] .title');
  await until(`document.body.dataset.mpage==='reader'`);
  await native('#reader-tags .tag-chip[data-tag-id]');
  await until(`document.body.dataset.listKind==='tag' && !!document.querySelector('#tags li.active')`);
  back();
  await until(`document.body.dataset.mpage==='articles'`);
  const tagView = await js(`({kind:document.body.dataset.listKind,tagId:document.querySelector('#tags li.active')?.dataset.tagId,hasTaggedEntry:!!document.querySelector('#entries li[data-id="1"]')})`);
  assert(tagView.kind==='tag' && tagView.hasTaggedEntry);
  record('tag-chip-opens-tag-view', tagView);

  writeFileSync(`${out}/results.json`, JSON.stringify({ serial, fixtureSha256:'03911253c9ff26ac7f1eb0d8790e0e9dc6c301b7f6455248d04dfeeb40eddd91', checks }, null, 2) + '\n');
  console.log(JSON.stringify({ checks:checks.map(c=>c.name) }));
} finally {
  socket.close();
}
