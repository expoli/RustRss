// Run on a task-owned API 36 emulator with the isolated reading fixture installed.
// ANDROID_SERIAL=emulator-5582 CDP_URL=http://127.0.0.1:9228/json node scripts/verify-reading-experience-menu.mjs OUT
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { resolve } from 'node:path';

const serial = process.env.ANDROID_SERIAL;
assert(serial, 'Select the task-owned emulator');
const out = resolve(process.argv[2]);
mkdirSync(out, { recursive: true });
const adb = (...args) => execFileSync('/usr/lib/android-sdk/platform-tools/adb', ['-s', serial, ...args], { timeout: 30000 });
const delay = ms => new Promise(r => setTimeout(r, ms));
const target = (await (await fetch(process.env.CDP_URL || 'http://127.0.0.1:9228/json')).json())
  .find(t => t.type === 'page' && JSON.parse(t.description || '{}').attached);
assert(target, 'Attached app WebView not found');
const socket = new WebSocket(target.webSocketDebuggerUrl);
await new Promise((resolveReady, reject) => { socket.onopen = resolveReady; socket.onerror = reject; });
let seq = 0;
const pending = new Map();
socket.onmessage = event => {
  const message = JSON.parse(event.data);
  const slot = pending.get(message.id);
  if (!slot) return;
  clearTimeout(slot.timer);
  pending.delete(message.id);
  message.error ? slot.reject(message.error) : slot.resolve(message.result);
};
const call = (method, params = {}) => new Promise((resolveCall, reject) => {
  const id = ++seq;
  const timer = setTimeout(() => { pending.delete(id); reject(new Error(`DevTools timeout: ${method}`)); }, 15000);
  pending.set(id, { resolve: resolveCall, reject, timer });
  socket.send(JSON.stringify({ id, method, params }));
});
async function js(expression) {
  const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  assert(!result.exceptionDetails, JSON.stringify(result.exceptionDetails));
  return result.result.value;
}
async function until(expression) {
  for (let i = 0; i < 80; i++) {
    if (await js(expression)) return;
    await delay(100);
  }
  throw new Error(`Timed out: ${expression}`);
}
async function click(selector) {
  await js(`document.querySelector(${JSON.stringify(selector)}).click()`);
  await delay(150);
}
async function action(label) {
  const found = await js(`(() => { const b=[...document.querySelectorAll('.ctx-sheet-list button')].find(b=>b.textContent.includes(${JSON.stringify(label)})); if(!b)return false; b.click(); return true; })()`);
  assert(found, `Menu action missing: ${label}`);
  await delay(200);
}
async function native(selector) {
  await js(`document.querySelector(${JSON.stringify(selector)}).scrollIntoView({block:'center'})`);
  const rect = await js(`(() => {const n=document.querySelector(${JSON.stringify(selector)}),r=n.getBoundingClientRect();const x=r.x+r.width/2,y=r.y+r.height/2;
    return {x,y,dpr:devicePixelRatio,visible:r.width>0&&r.height>0&&document.elementFromPoint(x,y)?.closest(${JSON.stringify(selector)})===n&&y<visualViewport.height}})()`);
  assert(rect.visible, `Native tap target clipped: ${selector} ${JSON.stringify(rect)}`);
  const y = JSON.parse(target.description).screenY + rect.y * rect.dpr;
  adb('shell', 'input', 'tap', String(Math.round(rect.x * rect.dpr)), String(Math.round(y)));
  await delay(350);
}
async function nativeHold(selector) {
  await js(`document.querySelector(${JSON.stringify(selector)}).scrollIntoView({block:'center'})`);
  const point = await js(`(() => {const n=document.querySelector(${JSON.stringify(selector)}),r=n.getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2,dpr:devicePixelRatio};})()`);
  const x = Math.round(point.x * point.dpr);
  const y = Math.round(JSON.parse(target.description).screenY + point.y * point.dpr);
  adb('shell', 'input', 'swipe', String(x), String(y), String(x), String(y), '750');
  await delay(250);
}
function back() { adb('shell', 'input', 'keyevent', 'KEYCODE_BACK'); }
function shot(name) { writeFileSync(`${out}/${name}.png`, adb('exec-out', 'screencap', '-p')); }
const checks = [];
function record(name, value) { checks.push({ name, ...value }); }
async function feed(id) { return (await js(`window.__TAURI__.core.invoke('list_feeds')`)).find(row => row.id === id); }
async function tags() { return js(`window.__TAURI__.core.invoke('list_tags',{recentFirst:false})`); }
async function deletionFocus(listId, removedKey) {
  return js(`(() => { const n=document.activeElement, row=n?.closest('li[data-key]'); return {
    active: n?.outerHTML.slice(0, 180), rowKey: row?.dataset.key || null,
    inList: row?.parentElement?.id === ${JSON.stringify(listId)},
    heading: n?.id === ${JSON.stringify(listId === 'feeds' ? 'feeds-heading' : 'tags-head')} || n?.id === 'list-title',
    visible: !!n?.getClientRects().length,
    removed: !!document.querySelector(${JSON.stringify(`#${listId} li[data-key="${removedKey}"]`)})
  }; })()`);
}
let server;
try {
  await until(`document.querySelectorAll('#feeds li[data-feed-id]').length === 3`);
  await native('[data-mpage-btn="subscriptions"]');
  assert.equal(await js(`document.body.dataset.mpage`), 'subscriptions');
  const before = await feed(2);
  await native('#feeds li[data-feed-id="2"] .row-more');
  assert.equal(await js(`document.body.dataset.mpage`), 'subscriptions');
  assert.equal(await js(`document.querySelector('#ctx-sheet-title').textContent`), before.title);
  const initial = await js(`(() => {const s=document.querySelector('.ctx-sheet-overlay').getBoundingClientRect(),v=visualViewport;
    return {screen:[v.width,v.height],sheet:[s.x,s.y,s.width,s.height],rows:[...document.querySelectorAll('.ctx-sheet-list button')].map(b=>({name:b.textContent,role:b.getAttribute('role'),h:b.getBoundingClientRect().height}))};})()`);
  record('native-row-more-and-sheet', initial);
  shot('01-feed-long-title');
  assert(initial.rows.every(row => row.h >= 48 && row.role === 'menuitem'));
  assert(initial.sheet[2] <= initial.screen[0] + 1 && initial.sheet[3] <= initial.screen[1] + 1);
  await action('刷新间隔');
  assert(await js(`document.querySelector('.ctx-sheet-list [aria-checked="true"]')?.textContent.includes('跟随全局')`));
  shot('02-interval-current');
  back(); await until(`document.querySelector('#ctx-sheet-title')?.textContent.includes('subscription')`);
  back(); await until(`!document.querySelector('#ctx-menu')`);
  assert.equal((await feed(2)).refresh_interval_minutes, before.refresh_interval_minutes);
  assert(await js(`document.activeElement === document.querySelector('#feeds li[data-feed-id="2"] .row-more')`));
  record('native-back-subpage-close-no-write-focus', { passed: true });
  await nativeHold('#feeds li[data-feed-id="2"] .row-select');
  await until(`!!document.querySelector('.ctx-sheet-overlay')`);
  assert.equal(await js(`document.body.dataset.mpage`), 'subscriptions');
  record('native-long-press-still-opens-row-menu', { title: await js(`document.querySelector('#ctx-sheet-title').textContent`), historyEntry: await js(`history.state?.rr`) });
  back(); await until(`!document.querySelector('#ctx-menu')`);
  record('native-long-press-back-closes-menu', { passed: true });
  await native('#feeds li[data-feed-id="2"] .row-select');
  await until(`document.querySelector('#list-title').textContent.includes('A deliberately long subscription')`);
  record('feed-row-select', { title: await js(`document.querySelector('#list-title').textContent`) });
  await native('[data-mpage-btn="subscriptions"]');
  await native('#feeds li[data-folder-id="1"] .name');
  await until(`document.querySelector('#list-title').textContent.includes('Folder 00')`);
  record('folder-aggregate-select', { title: await js(`document.querySelector('#list-title').textContent`) });
  await native('[data-mpage-btn="subscriptions"]');
  await native('#tags li[data-tag-id="1"] .row-select');
  await until(`document.querySelector('#list-title').textContent.includes('标签')`);
  record('tag-view-select', { title: await js(`document.querySelector('#list-title').textContent`) });
  await native('[data-mpage-btn="subscriptions"]');

  await native('#feeds li[data-feed-id="2"] .row-more');
  await action('刷新间隔');
  await action('15');
  await until(`window.__TAURI__.core.invoke('list_feeds').then(rows=>rows.find(r=>r.id===2).refresh_interval_minutes===15)`);
  record('interval-action-after-history', { interval: (await feed(2)).refresh_interval_minutes });
  // Both ungrouped feeds can move without dragging. Both directions persist.
  await native('#feeds li[data-feed-id="3"] .row-more');
  await action('在组内上移');
  await until(`window.__TAURI__.core.invoke('list_feeds').then(rows=>rows.findIndex(r=>r.id===3)<rows.findIndex(r=>r.id===2))`);
  await native('#feeds li[data-feed-id="3"] .row-more');
  await action('在组内下移');
  await until(`window.__TAURI__.core.invoke('list_feeds').then(rows=>rows.findIndex(r=>r.id===3)>rows.findIndex(r=>r.id===2))`);
  record('feed-touch-up-down', { ids: (await js(`window.__TAURI__.core.invoke('list_feeds')`)).filter(row => row.folder_id == null).map(row => row.id) });

  await native('#feeds li[data-feed-id="2"] .row-more');
  await action('移动到');
  const folderOptions = await js(`(() => {const n=document.querySelector('.ctx-sheet-list');return {items:n.querySelectorAll('button').length,scroll:n.scrollHeight>n.clientHeight,visibleHeight:n.clientHeight,totalHeight:n.scrollHeight};})()`);
  assert(folderOptions.items >= 25 && folderOptions.scroll);
  shot('02-folder-long-options');
  record('folder-options-internal-scroll', folderOptions);
  await action('Folder 00');
  await until(`window.__TAURI__.core.invoke('list_feeds').then(rows=>rows.find(r=>r.id===2).folder_id===1)`);
  record('move-target-stable', { folderId: (await feed(2)).folder_id });

  await click('#feeds li[data-folder-id="1"] .folder-arrow');
  assert(await js(`!document.querySelector('#feeds li[data-feed-id="1"]')`));
  await click('#feeds li[data-folder-id="1"] .folder-arrow');
  assert(await js(`!!document.querySelector('#feeds li[data-feed-id="1"]')`));
  record('folder-collapse-expand', { passed: true });
  await click('#tags-head');
  assert(await js(`document.querySelector('#tags').classList.contains('hidden')`));
  await click('#tags-head');
  assert(await js(`!document.querySelector('#tags').classList.contains('hidden')`));
  record('tags-collapse-expand', { passed: true });

  await native('#tags li[data-tag-id="1"] .row-more');
  await action('颜色');
  assert(await js(`document.querySelector('.ctx-sheet-list [aria-checked="true"]')?.textContent.includes('默认')`));
  shot('03-tag-color-current');
  await action('红');
  await until(`window.__TAURI__.core.invoke('list_tags',{recentFirst:false}).then(rows=>!!rows.find(r=>r.id===1).color)`);
  record('tag-color', { color: (await tags()).find(row => row.id === 1).color });
  await native('#tags li[data-tag-id="1"] .row-more');
  await action('颜色');
  await action('默认');
  await until(`window.__TAURI__.core.invoke('list_tags',{recentFirst:false}).then(rows=>rows.find(r=>r.id===1).color===null)`);
  record('tag-default-color', { color: (await tags()).find(row => row.id === 1).color });
  await native('#tags li[data-tag-id="1"] .row-more');
  await action('标签下移');
  await until(`window.__TAURI__.core.invoke('list_tags',{recentFirst:false}).then(rows=>rows.findIndex(r=>r.id===1)>rows.findIndex(r=>r.id===2))`);
  record('tag-touch-sort', { ids: (await tags()).map(row => row.id) });
  await native('#tags li[data-tag-id="1"] .row-more');
  await action('标签上移');
  await until(`window.__TAURI__.core.invoke('list_tags',{recentFirst:false}).then(rows=>rows.findIndex(r=>r.id===1)<rows.findIndex(r=>r.id===2))`);
  record('tag-touch-sort-reverse', { ids: (await tags()).map(row => row.id) });

  await native('#feeds li[data-folder-id="1"] .row-more');
  await action('删除');
  assert(await js(`!document.querySelector('#generic-confirm-overlay').classList.contains('hidden')`));
  shot('04-folder-delete-impact');
  await native('#generic-confirm-cancel');
  assert((await feed(2)).folder_id === 1);
  assert(await js(`document.activeElement === document.querySelector('#feeds li[data-folder-id="1"] .row-more')`));
  record('folder-delete-cancel-focus', { trigger: 'folder:1', restored: true });
  await native('#feeds li[data-folder-id="1"] .row-more');
  await action('删除');
  await native('#generic-confirm-ok');
  await until(`window.__TAURI__.core.invoke('list_feeds').then(rows=>rows.find(r=>r.id===2).folder_id===null)`);
  await until(`document.activeElement?.closest('li[data-key]')?.dataset.key !== 'h:1' && document.querySelector('#generic-confirm-overlay').classList.contains('hidden')`);
  const folderFocus = await deletionFocus('feeds', 'h:1');
  assert(!folderFocus.removed && folderFocus.visible && (folderFocus.inList || folderFocus.heading), JSON.stringify(folderFocus));
  record('folder-delete-preserves-feeds', { remainingFeedId: (await feed(2)).id, folderId: (await feed(2)).folder_id, focus: folderFocus });

  await click('#btn-new-folder');
  assert(await js(`!!document.querySelector('.prompt-overlay input')`));
  await js(`document.querySelector('.prompt-overlay input').value='T2 folder'`);
  await click('.prompt-overlay [data-act="ok"]');
  await until(`window.__TAURI__.core.invoke('list_folders').then(rows=>rows.some(r=>r.name==='T2 folder'))`);
  const createdFolder = (await js(`window.__TAURI__.core.invoke('list_folders')`)).find(row => row.name === 'T2 folder');
  await native(`#feeds li[data-folder-id="${createdFolder.id}"] .row-more`);
  await action('重命名');
  await js(`document.querySelector('.prompt-overlay input').value='T2 renamed'`);
  await click('.prompt-overlay [data-act="ok"]');
  await until(`window.__TAURI__.core.invoke('list_folders').then(rows=>rows.some(r=>r.id===${createdFolder.id}&&r.name==='T2 renamed'))`);
  record('folder-create-rename', { id: createdFolder.id });

  await native('#feeds li[data-feed-id="2"] .row-more');
  await action('编辑');
  assert(await js(`document.querySelector('#feed-edit-url').textContent.includes('/en.xml') && !document.querySelector('#feed-edit-url').isContentEditable`));
  await click('#feed-edit-cancel');
  assert.equal((await feed(2)).custom_title, before.custom_title);
  await native('#feeds li[data-feed-id="2"] .row-more');
  await action('编辑');
  await js(`document.querySelector('#feed-edit-name').value='T2 edited feed'`);
  await click('#feed-edit-save');
  await until(`window.__TAURI__.core.invoke('list_feeds').then(rows=>rows.find(r=>r.id===2).custom_title==='T2 edited feed')`);
  record('feed-edit-cancel-save-readonly-url', { title: (await feed(2)).title });
  await native('#feeds li[data-feed-id="2"] .row-more');
  await action('编辑');
  await click('#feed-edit-folder');
  await js(`(() => {[...document.querySelectorAll('#ctx-menu button')].find(b=>b.textContent.includes('T2 renamed')).click()})()`);
  assert(await js(`document.querySelector('#feed-edit-folder').textContent.includes('T2 renamed')`));
  await click('#feed-edit-interval');
  await js(`(() => {[...document.querySelectorAll('#ctx-menu button')].find(b=>b.textContent.includes('每 30 分钟')).click()})()`);
  await click('#feed-edit-save');
  await until(`window.__TAURI__.core.invoke('list_feeds').then(rows=>{const r=rows.find(r=>r.id===2);return r.folder_id===${createdFolder.id}&&r.refresh_interval_minutes===30})`);
  record('feed-edit-folder-interval', { folderId: (await feed(2)).folder_id, interval: (await feed(2)).refresh_interval_minutes });

  await native('#feeds li[data-feed-id="3"] .row-more');
  await action('取消订阅');
  assert(await js(`document.querySelector('#generic-confirm-body').textContent.includes('全部条目')`));
  await native('#generic-confirm-cancel');
  assert(await feed(3));
  assert(await js(`document.activeElement === document.querySelector('#feeds li[data-feed-id="3"] .row-more')`));
  record('unsubscribe-cancel-focus', { trigger: 'feed:3', restored: true });
  await native('#feeds li[data-feed-id="3"] .row-more');
  await action('取消订阅');
  await native('#generic-confirm-ok');
  await until(`window.__TAURI__.core.invoke('list_feeds').then(rows=>!rows.some(r=>r.id===3))`);
  const feedFocus = await deletionFocus('feeds', 'f:3');
  assert(!feedFocus.removed && feedFocus.visible && (feedFocus.inList || feedFocus.heading), JSON.stringify(feedFocus));
  record('unsubscribe-cancel-confirm', { removedFeedId: 3, preservedFeedId: (await feed(2)).id, focus: feedFocus });

  await native('#tags li[data-tag-id="2"] .row-more');
  await action('置顶');
  await until(`window.__TAURI__.core.invoke('list_tags',{recentFirst:false}).then(rows=>rows.find(r=>r.id===2).pinned)`);
  await native('#tags li[data-tag-id="2"] .row-more');
  await action('重命名');
  await js(`document.querySelector('.prompt-overlay input').value='T2 tag'`);
  await click('.prompt-overlay [data-act="ok"]');
  await until(`window.__TAURI__.core.invoke('list_tags',{recentFirst:false}).then(rows=>rows.find(r=>r.id===2).name==='T2 tag')`);
  record('tag-pin-rename', { tag: (await tags()).find(row => row.id === 2) });
  await native('#tags li[data-tag-id="2"] .row-more');
  await action('取消置顶');
  await until(`window.__TAURI__.core.invoke('list_tags',{recentFirst:false}).then(rows=>!rows.find(r=>r.id===2).pinned)`);
  record('tag-unpin', { tagId: 2, pinned: (await tags()).find(row => row.id === 2).pinned });

  await native('#tags li[data-tag-id="3"] .row-more');
  await action('删除');
  assert(await js(`!document.querySelector('#generic-confirm-overlay').classList.contains('hidden')`));
  await native('#generic-confirm-cancel');
  assert((await tags()).some(row => row.id === 3));
  assert(await js(`document.activeElement === document.querySelector('#tags li[data-tag-id="3"] .row-more')`));
  record('tag-delete-cancel-focus', { trigger: 'tag:3', restored: true });
  await native('#tags li[data-tag-id="3"] .row-more');
  await action('删除');
  await native('#generic-confirm-ok');
  await until(`window.__TAURI__.core.invoke('list_tags',{recentFirst:false}).then(rows=>!rows.some(r=>r.id===3))`);
  const tagFocus = await deletionFocus('tags', 't:3');
  assert(!tagFocus.removed && tagFocus.visible && (tagFocus.inList || tagFocus.heading), JSON.stringify(tagFocus));
  record('tag-delete-cancel-confirm', { deletedId: 3, focus: tagFocus });

  await native('[data-mpage-btn="articles"]');
  await click('#m-seg-articles [data-mview="all"]');
  await until(`document.querySelector('#entries li[data-id]')`);
  await click('#entries li[data-id]');
  await until(`document.body.dataset.mpage==='reader'`);
  await native('#act-tags');
  assert(await js(`!document.querySelector('#tag-picker-overlay').classList.contains('hidden')`));
  await js(`(() => {const n=document.querySelector('#tag-picker-input');n.value='设计';n.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  assert(await js(`!!document.querySelector('#tag-picker-list li[data-tag-id="1"]')`));
  await click('#tag-picker-list li[data-tag-id="1"]');
  await until(`document.querySelector('#reader-tags').textContent.includes('Tag A')`);
  await click('#tag-picker-list li[data-tag-id="1"]');
  await until(`!document.querySelector('#reader-tags').textContent.includes('Tag A')`);
  await click('#tag-picker-list li[data-tag-id="1"]');
  await until(`document.querySelector('#reader-tags').textContent.includes('Tag A')`);
  record('article-tag-picker-chinese-search-select', { query: '设计', tagId: 1 });
  await js(`(() => {const n=document.querySelector('#tag-picker-input');n.value='T2 new tag';n.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  await click('#tag-picker-list li.create');
  await until(`window.__TAURI__.core.invoke('list_tags',{recentFirst:false}).then(rows=>rows.some(r=>r.name==='T2 new tag'))`);
  const newTag = (await tags()).find(row => row.name === 'T2 new tag');
  assert(await js(`document.querySelector('#reader-tags').textContent.includes('T2 new tag')`));
  await js(`(() => {const n=document.querySelector('#tag-picker-input');n.value='T2';n.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  assert(await js(`!!document.querySelector('#tag-picker-list li[data-tag-id="${newTag.id}"]')`));
  record('article-tag-picker-search-results', { query: 'T2', tagId: newTag.id });
  await click(`#tag-picker-list li[data-tag-id="${newTag.id}"]`);
  await until(`!document.querySelector('#reader-tags').textContent.includes('T2 new tag')`);
  assert((await tags()).some(row => row.id === newTag.id));
  await click(`#tag-picker-list li[data-tag-id="${newTag.id}"]`);
  await until(`document.querySelector('#reader-tags').textContent.includes('T2 new tag')`);
  await click('#tag-picker-close');
  await click(`#reader-tags .tag-chip[data-tag-id="${newTag.id}"]`);
  await until(`document.querySelector('#list-title').textContent.includes('T2 new tag')`);
  record('article-tag-chip-opens-view', { tagId: newTag.id });
  record('article-tag-picker-create-attach-remove', { tagId: newTag.id });
  await click('#m-reader-back');
  await native('[data-mpage-btn="subscriptions"]');

  const rsshubHits = [];
  server = createServer((request, response) => {
    if (request.url === '/site') {
      response.writeHead(200, { 'Content-Type': 'text/html' });
      response.end('<link rel="alternate" type="application/rss+xml" href="/one.xml"><link rel="alternate" type="application/rss+xml" href="/two.xml">');
    } else if (request.url === '/t2/menu-fixture') {
      rsshubHits.push(request.url);
      response.writeHead(200, { 'Content-Type': 'application/rss+xml' });
      response.end('<?xml version="1.0"?><rss version="2.0"><channel><title>T2 RSSHub fixture</title><link>http://example.invalid/</link><description>Local RSSHub mirror fixture</description><item><title>RSSHub local article</title><guid>rsshub-t2-1</guid><description>Body</description></item></channel></rss>');
    } else if (request.url?.endsWith('.xml')) {
      response.writeHead(200, { 'Content-Type': 'application/rss+xml' });
      response.end(`<?xml version="1.0"?><rss version="2.0"><channel><title>${request.url}</title><link>http://example.invalid/</link><description>Fixture</description><item><title>T2 local article</title><guid>${request.url}</guid><description>Body</description></item></channel></rss>`);
    } else { response.writeHead(404); response.end(); }
  });
  await new Promise(resolveReady => server.listen(0, '0.0.0.0', resolveReady));
  const site = `http://10.0.2.2:${server.address().port}/site`;
  await js(`document.querySelector('#add-url').value=${JSON.stringify(site)}`);
  await click('#add-ok');
  await until(`document.querySelector('#ctx-sheet-title')?.textContent==='选择订阅源'`);
  assert.equal(await js(`document.querySelectorAll('.ctx-sheet-list button').length`), 2);
  shot('05-discovery-candidates');
  await action('two.xml');
  await until(`window.__TAURI__.core.invoke('list_feeds').then(rows=>rows.some(r=>r.url.endsWith('/two.xml')))`);
  record('url-discovery-candidate-add', { url: (await js(`window.__TAURI__.core.invoke('list_feeds')`)).find(row => row.url.endsWith('/two.xml')).url });
  const added = (await js(`window.__TAURI__.core.invoke('list_feeds')`)).find(row => row.url.endsWith('/two.xml'));
  await native(`#feeds li[data-feed-id="${added.id}"] .row-more`);
  await action('立即刷新');
  await until(`window.__TAURI__.core.invoke('list_feeds').then(rows=>rows.find(r=>r.id===${added.id}).last_status==='ok')`);
  assert.equal((await feed(added.id)).url, added.url);
  record('refresh-feed-stable-id', { id: added.id, url: added.url, status: (await feed(added.id)).last_status });
  const mirror = `http://10.0.2.2:${server.address().port}`;
  assert.equal(await js(`window.__TAURI__.core.invoke('set_rsshub_mirror',{mirror:${JSON.stringify(mirror)}})`), mirror);
  await click('#add-rsshub');
  assert.equal(await js(`document.querySelector('#add-url').value`), 'rsshub://');
  const rsshubUrl = 'rsshub://t2/menu-fixture';
  await js(`document.querySelector('#add-url').value=${JSON.stringify(rsshubUrl)}`);
  await native('#add-ok');
  await until(`window.__TAURI__.core.invoke('list_feeds').then(rows=>rows.some(r=>r.url==='rsshub://t2/menu-fixture'&&r.last_status==='ok'))`);
  const rsshubFeed = (await js(`window.__TAURI__.core.invoke('list_feeds')`)).find(row => row.url === rsshubUrl);
  assert(rsshubHits.includes('/t2/menu-fixture'));
  assert.equal(rsshubFeed.title, 'T2 RSSHub fixture');
  record('rsshub-full-address-add-result', { input: rsshubUrl, mirror, requestedPaths: rsshubHits, feedId: rsshubFeed.id, storedUrl: rsshubFeed.url, title: rsshubFeed.title, status: rsshubFeed.last_status });
  await click('#add-rsshub');
  await native('#add-url');
  await until(`visualViewport.height < 700`);
  const ime = await js(`(() => {const v=visualViewport,r=document.querySelector('#add-url').getBoundingClientRect();return {visual:[v.width,v.height],input:[r.top,r.bottom],visible:r.bottom<=v.height,overflow:document.documentElement.scrollWidth>v.width+1};})()`);
  assert(ime.visible && !ime.overflow);
  shot('06-add-ime');
  record('native-ime-safe-area', ime);
  back();
  await js(`document.querySelector('#add-url').value='not-a-url'`);
  await click('#add-ok');
  await until(`!document.querySelector('#add-ok').disabled`);
  assert.equal(await js(`document.querySelector('#add-url').value`), 'not-a-url');
  assert(await js(`document.querySelector('#status').classList.contains('error')`));
  record('add-failure-retains-input', { input: 'not-a-url' });

  for (const width of [360, 412]) {
    await call('Emulation.setDeviceMetricsOverride', { width, height: 800, deviceScaleFactor: 1, mobile: true });
    await click('#feeds li[data-feed-id="2"] .row-more');
    const layout = await js(`(() => {const s=document.querySelector('.ctx-sheet-overlay').getBoundingClientRect(); const h=document.querySelector('.ctx-sheet-head').getBoundingClientRect();
      return {visual:[visualViewport.width,visualViewport.height],sheet:[s.x,s.y,s.right,s.bottom],header:[h.x,h.y,h.right,h.bottom],scroll:document.querySelector('.ctx-sheet-list').scrollHeight>document.querySelector('.ctx-sheet-list').clientHeight,overflow:document.documentElement.scrollWidth>visualViewport.width+1};})()`);
    assert(layout.sheet[0] >= -1 && layout.sheet[2] <= layout.visual[0] + 1 && layout.sheet[3] <= layout.visual[1] + 1 && !layout.overflow);
    shot(`05-menu-${width}`);
    record(`webview-width-${width}`, layout);
    await click('.ctx-sheet-close');
  }
  await call('Emulation.clearDeviceMetricsOverride');
  await click('#tags-head');
  assert(await js(`document.querySelector('#tags').classList.contains('hidden')`));
  record('tags-collapse-before-restart', { collapsed: true });
  writeFileSync(`${out}/results.json`, JSON.stringify({ serial, apk: 'debug-x86_64', checks }, null, 2) + '\n');
  console.log(JSON.stringify({ out, checks: checks.length }));
} catch (error) {
  writeFileSync(`${out}/results.json`, JSON.stringify({ serial, checks, error: String(error.stack || error) }, null, 2) + '\n');
  throw error;
} finally {
  server?.close();
  await call('Emulation.clearDeviceMetricsOverride').catch(() => {});
  socket.close();
}
