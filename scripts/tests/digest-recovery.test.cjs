const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync('ui/app.js', 'utf8');
function extract(name) {
  const start = source.indexOf(`function ${name}(`);
  const end = source.indexOf('\n}', start) + 2;
  return (source.slice(start - 6, start) === 'async ' ? 'async ' : '') + source.slice(start, end);
}
function context(saved) {
  const storage = new Map(saved ? [['ui.location', JSON.stringify(saved)]] : []);
  const calls = [], timers = new Map(); let timerId = 0;
  const nodes = new Map();
  const el = id => {
    if (!nodes.has(id)) nodes.set(id, { innerHTML: '', classList: { remove() {} }, querySelectorAll: () => [], addEventListener() {} });
    return nodes.get(id);
  };
  const c = vm.createContext({
    sessionStorage: { getItem: key => storage.get(key), setItem: (key, value) => storage.set(key, value) },
    locationReady: false, digestOpenDate: '2026-10-04', digestOpenScope: 'tags:1', digestJob: null,
    digestPollTimer: null, digestPollSerial: 0, readerToken: 1, chatView: null, state: {}, el,
    window: { RustRssMobileDigest: { restoreHome: () => calls.push('home') } },
    document: { querySelector: () => ({ classList: { contains: () => false } }) },
    t: key => key, escapeHtml: String, leaveChatView() {}, renderReaderEmpty() {},
    digestDate: kind => kind === 'today' ? '2026-10-04' : '2026-10-03', digestScopeTags: () => [],
    invoke: async (cmd, args) => { calls.push({ cmd, args }); return { generating: true }; },
    openDigestDate: async (date, scope) => calls.push({ date, scope }),
    renderChatView: async id => calls.push({ chat: id }), refreshSidebarDigestDays: () => calls.push('sidebar'), setStatus() {},
    setTimeout: (fn, delay) => { assert.equal(delay, 3000); const id = ++timerId; timers.set(id, fn); return id; },
    clearTimeout: id => timers.delete(id),
  });
  for (const name of ['rememberLocation', 'rememberMobilePage', 'restoreLocation', 'stopDigestPolling', 'startDigestPolling', 'onDigestDone']) vm.runInContext(extract(name), c);
  return { c, storage, calls, timers, tick: async () => { assert.equal(timers.size, 1); const [id, fn] = timers.entries().next().value; timers.delete(id); await fn(); } };
}

test('startup default writes do not overwrite the remembered digest; restore home then exact date/scope', async () => {
  const { c, storage, calls } = context({ mpage: 'digest', digestDate: '2026-10-04', digestScope: 'tags:1' });
  c.rememberLocation('articles');
  assert.equal(JSON.parse(storage.get('ui.location')).mpage, 'digest');
  await c.restoreLocation();
  assert.equal(calls[0].cmd, 'digest_get');
  assert.equal(calls[1], 'home');
  assert.deepEqual(calls[2], { date: '2026-10-04', scope: 'tags:1' });
});

test('digest home and desktop/chat session are restored without requiring mobile navigation', async () => {
  const home = context({ mpage: 'digest' }); await home.c.restoreLocation();
  assert.deepEqual(home.calls, ['home']);
  const chat = context({ mpage: 'chat', chatSessionId: 7 }); await chat.c.restoreLocation();
  assert.equal(chat.calls[0].cmd, 'chat_session_get'); assert.deepEqual(chat.calls[1], { chat: 7 });
  const fresh = context({ mpage: 'chat', chatSessionId: null }); await fresh.c.restoreLocation();
  assert.deepEqual(fresh.calls, [{ chat: undefined }]);
});

test('deleted data, malformed JSON, and unavailable storage fall back silently', async () => {
  for (const saved of [{ mpage: 'chat', chatSessionId: 7 }, { mpage: 'digest', digestDate: '2026-01-01' }]) {
    const x = context(saved); x.c.invoke = async () => null;
    await x.c.restoreLocation();
    assert.equal(JSON.parse(x.storage.get('ui.location')).mpage, 'articles'); assert.equal(x.calls.length, 0);
  }
  const malformed = context(); malformed.storage.set('ui.location', '{'); await malformed.c.restoreLocation();
  assert.equal(JSON.parse(malformed.storage.get('ui.location')).mpage, 'articles');
  const denied = context(); denied.c.sessionStorage = { getItem() { throw Error('denied'); }, setItem() { throw Error('denied'); } };
  await denied.c.restoreLocation(); denied.c.rememberLocation('digest');
});

test('mobile reader retains digest route; other pages clear date and chat memory', () => {
  const x = context(); x.c.locationReady = true;
  x.c.rememberMobilePage('reader');
  assert.deepEqual(JSON.parse(x.storage.get('ui.location')), { mpage: 'digest', digestDate: '2026-10-04', digestScope: 'tags:1', chatSessionId: null });
  x.c.rememberMobilePage('saved');
  assert.deepEqual(JSON.parse(x.storage.get('ui.location')), { mpage: 'saved', digestDate: null, digestScope: null, chatSessionId: null });
});

test('reloaded generating view with no digestJob shows placeholder and starts one poll', () => {
  const x = context(); vm.runInContext(extract('renderDigestView'), x.c);
  x.c.renderDigestView({ date: '2026-10-04', scope_key: 'tags:1', generating: true, status: { candidate_count: 2 }, sections: [] });
  assert.match(x.c.el('reader').innerHTML, /digest.generating/);
  assert.match(x.c.el('reader').innerHTML, /id="digest-cancel" disabled/);
  assert.equal(x.timers.size, 1); assert.equal(x.c.digestJob, null);
});

test('polls every 3s without reading body until persisted completion, even when done event was lost', async () => {
  const x = context(); x.c.startDigestPolling('2026-10-04', 'tags:1');
  await x.tick(); assert.equal(x.calls[0].cmd, 'digest_status'); assert.equal(x.calls[0].args.scopeKey, 'tags:1'); assert.equal(x.timers.size, 1);
  x.c.invoke = async () => ({ generating: false });
  await x.tick(); assert.deepEqual(x.calls[1], { date: '2026-10-04', scope: 'tags:1' }); assert.equal(x.calls[2], 'sidebar'); assert.equal(x.timers.size, 0);
});

test('poll never overlaps, transient failures retry, stale response cannot steal navigation', async () => {
  const x = context(); let resolve;
  x.c.invoke = () => new Promise(r => { resolve = r; });
  x.c.startDigestPolling('2026-10-04', 'tags:1');
  const pending = x.tick(); assert.equal(x.timers.size, 0);
  x.c.readerToken++; resolve({ generating: false }); await pending;
  assert.equal(x.calls.length, 0); assert.equal(x.timers.size, 0);
  x.c.invoke = async () => { throw Error('offline'); };
  x.c.startDigestPolling('2026-10-04', 'tags:1'); await x.tick(); assert.equal(x.timers.size, 1);
  x.c.stopDigestPolling(); assert.equal(x.timers.size, 0);
});

test('done event wakes a reloaded view immediately, keeping scope; unrelated date never navigates', () => {
  const x = context(); x.c.startDigestPolling('2026-10-04', 'tags:1');
  x.c.onDigestDone({ payload: { date: '2026-10-02', jobId: 'old', ok: false } }); assert.equal(x.calls.length, 0);
  x.c.onDigestDone({ payload: { date: '2026-10-04', jobId: 'reloaded', ok: true } });
  assert.deepEqual(x.calls[0], { date: '2026-10-04', scope: 'tags:1' }); assert.equal(x.timers.size, 0);
});

test('initialization restores only after sidebar days; mobile setPage and digest home write location', () => {
  const boot = extract('boot');
  assert.match(boot, /initRefreshEvents\(\);\s+await refreshSidebarDigestDays\(\);\s+await restoreLocation\(\);/);
  const mobile = fs.readFileSync('ui/mobile.js', 'utf8');
  const setPage = mobile.slice(mobile.indexOf('  function setPage('), mobile.indexOf('  function syncNav('));
  assert.match(setPage, /RustRssLocation\?\.page\(name\)/);
  const home = mobile.slice(mobile.indexOf('  function renderDigestHome('), mobile.indexOf('  window.RustRssMobileDigest'));
  assert.match(home, /RustRssLocation\?\.digestHome\(\)/);
});

test('restarting polling on the same view invalidates the previous in-flight request', async () => {
  const x = context(); let resolve;
  x.c.invoke = () => new Promise(r => { resolve = r; });
  x.c.startDigestPolling('2026-10-04', 'tags:1');
  const old = x.tick();
  x.c.startDigestPolling('2026-10-04', 'tags:1');
  resolve({ generating: true }); await old;
  assert.equal(x.timers.size, 1, 'old request must not create a second timer');
  x.c.stopDigestPolling();
});

test('known local job keeps cancellation enabled when rendering backend generating state', () => {
  const x = context(); x.c.digestJob = { date: '2026-10-04', jobId: 'known-job' };
  vm.runInContext(extract('renderDigestView'), x.c);
  x.c.renderDigestView({ date: '2026-10-04', scope_key: 'tags:1', generating: true, status: { candidate_count: 2 }, sections: [] });
  assert.doesNotMatch(x.c.el('reader').innerHTML, /id="digest-cancel" disabled/);
});

test('generating=false then a failed digest_get re-arms polling and succeeds on retry', async () => {
  const x = context();
  const deferred = () => {
    let resolve, reject;
    const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
    return { promise, resolve, reject };
  };
  const reads = [deferred(), deferred()], started = [deferred(), deferred()];
  let digestGets = 0;
  x.c.renderList = () => {}; x.c.renderSidebar = () => {};
  for (const name of ['openDigestDate', 'renderDigestView']) vm.runInContext(extract(name), x.c);
  x.c.invoke = async (cmd, args) => {
    x.calls.push({ cmd, args });
    if (cmd === 'digest_status') return { generating: false };
    if (cmd === 'digest_get') {
      const index = digestGets++;
      assert.ok(index < reads.length, 'only the failed read and its retry are expected');
      started[index].resolve();
      return reads[index].promise;
    }
    throw Error('unexpected cmd ' + cmd);
  };
  x.c.renderDigestView({ date: '2026-10-04', scope_key: 'tags:1', generating: true, status: { candidate_count: 2 }, sections: [] });
  const placeholder = x.c.el('reader').innerHTML;
  const first = x.tick();
  await started[0].promise;
  assert.deepEqual(x.calls.map(c => c.cmd), ['digest_status', 'digest_get']);
  assert.equal(x.timers.size, 0, 'no overlapping poll while digest_get is pending');
  reads[0].reject(Error('transient db'));
  await first;
  assert.equal(digestGets, 1);
  assert.equal(x.timers.size, 1, 'failed digest_get must re-arm polling');
  assert.equal(x.c.el('reader').innerHTML, placeholder, 'failed read retains the generating view');
  assert.ok(!x.calls.includes('sidebar'), 'recovery is not complete after a failed read');

  const retry = x.tick();
  await started[1].promise;
  assert.deepEqual(x.calls.map(c => c.cmd), ['digest_status', 'digest_get', 'digest_status', 'digest_get']);
  for (const call of x.calls) {
    assert.equal(call.args.date, '2026-10-04');
    assert.equal(call.args.scopeKey, 'tags:1');
  }
  assert.equal(x.timers.size, 0, 'retry must also be single-flight');
  assert.equal(x.c.el('reader').innerHTML, placeholder, 'render waits for the successful read');
  reads[1].resolve({ date: '2026-10-04', scope_key: 'tags:1', generating: false, has_report: true,
    status: { candidate_count: 2, has_report: true }, sections: [], markdown: 'Recovered digest body' });
  await retry;
  assert.equal(digestGets, 2, 'digest_get succeeded on its retry');
  assert.match(x.c.el('reader').innerHTML, /Recovered digest body/);
  assert.doesNotMatch(x.c.el('reader').innerHTML, /digest.generating/);
  assert.equal(x.calls.filter(c => c === 'sidebar').length, 1);
  assert.equal(x.timers.size, 0, 'successful rendering ends polling');
  assert.equal(x.c.digestPollTimer, null);
});
