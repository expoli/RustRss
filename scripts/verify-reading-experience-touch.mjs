// Isolated browser check of the production CSS at a 360x800 coarse-pointer viewport.
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const output = resolve(process.argv[2] || 'reading-touch.json');
const root = await mkdtemp(join(tmpdir(), 'rustrss-reading-touch-'));
const ui = resolve('ui');
let html = await readFile(join(ui, 'index.html'), 'utf8');
html = html.replace('<head>', `<head><base href="${pathToFileURL(ui + '/').href}">`)
  .replace('<script src="app.js"></script>', '')
  .replace('<script src="mobile.js"></script>', '');
const page = join(root, 'fixture.html');
await writeFile(page, html);
const chrome = spawn('google-chrome', [
  '--headless=new', '--no-sandbox', '--disable-gpu', '--disable-extensions', '--no-first-run',
  '--remote-debugging-port=0', '--window-size=360,800', `--user-data-dir=${join(root, 'profile')}`,
  pathToFileURL(page).href,
], { stdio: 'ignore' });
let ws;
try {
  let port;
  for (let i = 0; i < 100; i++) {
    try { port = (await readFile(join(root, 'profile', 'DevToolsActivePort'), 'utf8')).split('\n')[0]; break; }
    catch { await new Promise(r => setTimeout(r, 100)); }
  }
  if (!port) throw new Error('Chrome debug port did not open');
  let target;
  for (let i = 0; i < 100; i++) {
    target = (await (await fetch(`http://127.0.0.1:${port}/json/list`)).json()).find(t => t.type === 'page' && t.url.includes('fixture.html'));
    if (target) break;
    await new Promise(r => setTimeout(r, 100));
  }
  if (!target) throw new Error('Fixture tab did not open');
  ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolveReady, reject) => { ws.addEventListener('open', resolveReady, { once: true }); ws.addEventListener('error', reject, { once: true }); });
  const pending = new Map(); let nextId = 0;
  ws.addEventListener('message', event => {
    const message = JSON.parse(event.data);
    if (!message.id) return;
    const slot = pending.get(message.id);
    if (!slot) return;
    pending.delete(message.id);
    message.error ? slot.reject(new Error(JSON.stringify(message.error))) : slot.resolve(message.result);
  });
  const call = (method, params = {}) => new Promise((resolveCall, reject) => {
    const id = ++nextId;
    pending.set(id, { resolve: resolveCall, reject });
    ws.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async expression => {
    const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.text);
    return result.result.value;
  };
  await call('Page.enable');
  await call('Emulation.setDeviceMetricsOverride', { width: 360, height: 800, deviceScaleFactor: 1, mobile: true });
  await call('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 5 });
  await call('Page.reload', { ignoreCache: true });
  for (let i = 0; i < 100; i++) {
    if (await evaluate('document.readyState === "complete" && !!window.I18N && !!window.RustRssIcons')) break;
    await new Promise(r => setTimeout(r, 100));
  }
  const metrics = await evaluate(`(() => {
    I18N.applyStaticI18n();
    document.body.dataset.mpage = 'articles';
    document.querySelector('#entries').innerHTML = '<li><span class="title">阅读与 Reading</span><span class="meta">Local fixture</span><button class="row-tag-btn" aria-label="Tag">+</button></li>';
    const rect = selector => {
      const el = document.querySelector(selector);
      const r = el.getBoundingClientRect();
      return { selector, width: r.width, height: r.height, visible: r.width > 0 && r.height > 0 };
    };
    const rows = ['#m-nav .m-nav-btn', '#m-seg-articles button', '#btn-list-sort', '#btn-list-bulk', '#btn-refresh', '#entries li', '#entries .row-tag-btn'].map(rect);
    document.body.dataset.mpage = 'reader';
    document.querySelector('#reader').innerHTML = '<div class="reader-head"><div class="meta"><span class="tag-bar"><button class="tag-add">+ Tag</button></span></div></div><div class="reader-actions"><button id="fixture-star">Star</button><button id="fixture-later">Later</button><button id="fixture-more">More</button></div>';
    rows.push(rect('#m-reader-back'), rect('#reader .tag-add'), rect('#fixture-star'), rect('#fixture-later'), rect('#fixture-more'));
    document.body.dataset.mpage = 'settings';
    const overlay = document.querySelector('#settings-overlay');
    document.querySelector('main').append(overlay); overlay.classList.remove('hidden');
    rows.push(rect('#settings-nav-list button'));
    document.querySelector('.settings-dialog').dataset.screen = 'detail';
    document.querySelector('#pane-subscriptions').classList.remove('hidden');
    rows.push(rect('#pane-subscriptions .switch'));
    overlay.classList.add('hidden');
    document.body.dataset.mpage = 'articles';
    const menu = document.createElement('div'); menu.className = 'ctx-menu'; menu.innerHTML = '<button>Action</button>'; document.body.append(menu);
    rows.push(rect('.ctx-menu button'));
    menu.remove();
    return { viewport: [innerWidth, innerHeight], pointerCoarse: matchMedia('(pointer: coarse)').matches, rows };
  })()`);
  if (!metrics.pointerCoarse || metrics.viewport[0] !== 360) throw new Error(`Mobile emulation unavailable: ${JSON.stringify(metrics)}`);
  const failed = metrics.rows.filter(row => !row.visible || row.width < 48 || row.height < 48);
  await writeFile(output, JSON.stringify({ ...metrics, failures: failed }, null, 2) + '\n');
  if (failed.length) throw new Error(`Touch targets below 48 CSS px: ${JSON.stringify(failed)}`);
  const shot = await call('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false });
  await writeFile(output.replace(/\.json$/, '.png'), Buffer.from(shot.data, 'base64'));
  console.log(JSON.stringify({ output, targets: metrics.rows.length, viewport: metrics.viewport, pointerCoarse: metrics.pointerCoarse }));
} finally {
  ws?.close();
  chrome.kill();
  if (chrome.exitCode === null) await new Promise(resolveExit => chrome.once('exit', resolveExit));
  await rm(root, { recursive: true, force: true, maxRetries: 5, retryDelay: 100 });
}
