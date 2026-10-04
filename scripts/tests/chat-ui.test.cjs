const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync('ui/app.js', 'utf8');
function extract(name, text = source) {
  const start = text.indexOf(`function ${name}(`);
  const end = text.indexOf('\n}', start) + 2;
  return (text.slice(start - 6, start) === 'async ' ? 'async ' : '') + text.slice(start, end);
}
function makeContext() {
  const nodes = new Map();
  const classes = new Set(['chat-active']);
  const right = { classList: { contains: x => classes.has(x), add: x => classes.add(x), remove: x => classes.delete(x) } };
  const el = id => {
    if (!nodes.has(id)) nodes.set(id, { value: '', textContent: '', innerHTML: '', hidden: false,
      querySelectorAll: () => [], addEventListener(event, fn) { this[event] = fn; } });
    return nodes.get(id);
  };
  const calls = [], store = new Map();
  const context = vm.createContext({
    el, document: { querySelector: () => right }, state: { selectedId: 42, readerEntry: { id: 42 } },
    chatView: null, chatRefreshSerial: 0, readerToken: 0, digestOpenDate: '2026-10-04',
    chatDrafts: new Map(), t: (key, args) => key + (args ? JSON.stringify(args) : ''),
    escapeHtml: x => String(x).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;'),
    localStorage: { getItem: k => store.get(k), setItem: (k, v) => store.set(k, v) },
    confirmDialog: async () => true, paintChatView: () => {}, renderList: () => {}, renderSidebar: () => {},
    refreshChatView: async view => { view.data = { session: { scope_json: JSON.stringify({ date: view.date, scope_key: view.scopeKey }) }, messages: [] }; }, refreshChatHistory: async () => {},
    openSettings: () => {}, showPane: () => {}, window: {},
    invoke: async (command, args) => {
      calls.push({ command, args });
      if (command === 'get_ai_settings') return context.ai;
      if (command === 'chat_send') return { sessionId: 7, historyTrimmed: false };
    },
    ai: { provider: 'openai', model: 'test', base_url: 'https://example.test' },
  });
  for (const name of ['chatScope', 'chatText', 'chatScopeLabel', 'chatIsVisible', 'chatError', 'sendChat', 'onChatTerminal']) vm.runInContext(extract(name), context);
  context.calls = calls; context.store = store; context.right = right;
  context.view = { id: null, pending: false, draft: '' };
  context.chatView = context.view;
  return context;
}

test('chat renders actual parts_json text blocks, not arbitrary HTML/tools', () => {
  const c = makeContext();
  assert.equal(c.chatText({ parts_json: '[{"text":"<script>bad</script>\\nline"},{"tool_call":{"name":"foo"}}]' }), '<script>bad</script>\nline');
  assert.equal(c.chatText({ parts_json: 'invalid' }), '');
  vm.runInContext(extract('buildChatMessage'), c);
  c.document.createElement = () => ({ classList: { toggle() {} } });
  const row = c.buildChatMessage({ role: 'assistant', status: 'done', parts_json: '[{"text":"<img src=x>\\ntext"}]' }, false, '', c.view);
  assert.equal(row.innerHTML, '<div>&lt;img src=x&gt;<br>text</div>');
  assert.match(c.buildChatMessage({ role: 'user', status: 'done' }, true, '', c.view).innerHTML, /chat.digestContext/);
});

test('send binds digest date/scope; privacy is remembered per endpoint and scope', async () => {
  const c = makeContext(); let confirmations = 0;
  c.confirmDialog = async args => { confirmations++; assert.match(args.body, /example.test/); return true; };
  c.view.date = '2026-10-03'; c.view.scopeKey = 'tags:4'; c.el('chat-input').value = 'question';
  await c.sendChat(c.view);
  const args = c.calls.find(call => call.command === 'chat_send').args;
  assert.equal(args.date, '2026-10-03'); assert.equal(args.scopeKey, 'tags:4'); assert.equal(args.sessionId, null);
  await c.sendChat(c.view, 'followup'); assert.equal(confirmations, 1);
  c.view.data.session.scope_json = '{}';
  await c.sendChat(c.view, 'wider'); assert.equal(confirmations, 2);
  c.ai.model = 'changed'; await c.sendChat(c.view, 'changed endpoint'); assert.equal(confirmations, 3);
});

test('privacy cancellation retains draft and never sends', async () => {
  const c = makeContext(); c.confirmDialog = async () => false; c.el('chat-input').value = 'draft';
  await c.sendChat(c.view);
  assert.equal(c.calls.some(x => x.command === 'chat_send'), false);
  assert.equal(c.el('chat-input').value, 'draft'); assert.equal(c.view.pending, false); assert.equal(c.store.size, 0);
});

test('single-flight submit and running stop do not resend', async () => {
  const c = makeContext(); c.view.pending = true;
  await c.sendChat(c.view, 'question'); assert.equal(c.calls.length, 0);
  c.view.pending = false; c.view.running = true; c.view.id = 7; c.view.data = { session: { scope_json: '{}' }, messages: [] };
  await c.sendChat(c.view, 'question'); assert.equal(c.calls[0].command, 'chat_stop'); assert.equal(c.calls.length, 1);
});

test('configuration/setup and budget errors retain draft and exact backend error', async () => {
  for (const error of ['未配置 API key', '达到会话预算，请开启新会话', '会话绑定的 AI 配置已变更，请开启新会话']) {
    const c = makeContext(); const original = c.invoke;
    c.invoke = async (...args) => { if (args[0] === 'chat_send') throw new Error(error); return original(...args); };
    c.el('chat-input').value = 'draft'; await c.sendChat(c.view);
    assert.equal(c.view.error, error); assert.equal(c.el('chat-input').value, 'draft'); assert.equal(c.view.pending, false);
    assert.equal(c.view.needSetup, error.includes('key'));
  }
});

test('completion never steals navigation, including events for another session', () => {
  const c = makeContext(); let refreshes = 0; c.refreshChatView = () => { refreshes++; };
  c.view.id = 7; c.onChatTerminal({ payload: { sessionId: 8 } }); assert.equal(refreshes, 0);
  c.right.classList.remove('chat-active'); c.onChatTerminal({ payload: { sessionId: 7 } }); assert.equal(refreshes, 0);
  c.right.classList.add('chat-active'); c.onChatTerminal({ payload: { sessionId: 7, error: 'failure' } });
  assert.equal(refreshes, 1); assert.equal(c.view.error, 'failure');
});

test('late session fetch cannot paint a replaced chat/article', async () => {
  const c = makeContext(); vm.runInContext(extract('refreshChatView'), c);
  c.view.id = 7; let resolve; let paints = 0;
  c.invoke = () => new Promise(r => { resolve = r; }); c.paintChatView = () => { paints++; };
  const promise = c.refreshChatView(c.view); c.chatView = { id: 8 };
  resolve({ session: { id: 7 }, messages: [] }); await promise;
  assert.equal(paints, 0); assert.equal(c.view.data, undefined);
});

test('persisted fast failure exposes setup and recovers the submitted draft', async () => {
  const c = makeContext(); vm.runInContext(extract('refreshChatView'), c);
  c.view.id = 7; c.view.lastText = 'submitted draft';
  c.invoke = async () => ({ session: { id: 7 }, messages: [{ role: 'assistant', status: 'failed', parts_json: '[{"text":"未配置 API key"}]' }] });
  await c.refreshChatView(c.view);
  assert.equal(c.view.needSetup, true); assert.equal(c.el('chat-input').value, 'submitted draft');
  assert.equal(c.chatDrafts.get(7), 'submitted draft');
});

test('successful send clears the unbound draft cache', async () => {
  const c = makeContext(); c.chatDrafts.set(null, 'old draft');
  await c.sendChat(c.view, 'old draft');
  assert.equal(c.chatDrafts.has(null), false);
});

test('message reconciliation preserves unchanged bubbles and never scrolls an up-scrolled user', () => {
  const c = makeContext(); vm.runInContext(extract('paintChatView'), c);
  c.currentLocale = () => 'en'; c.setText = (node, text) => { if (node.textContent !== text) node.textContent = text; };
  for (const id of ['chat-error', 'chat-send', 'chat-delete', 'chat-input', 'chat-setup', 'chat-retry', 'chat-heading', 'chat-meta']) c.el(id).classList = { toggle() {} };
  const list = c.el('chat-messages'); list.children = []; list.scrollHeight = 1000; list.scrollTop = 200; list.clientHeight = 400;
  list.insertBefore = (row, before) => { list.children.splice(before ? list.children.indexOf(before) : list.children.length, 0, row); };
  let builds = 0;
  c.buildChatMessage = () => {
    builds++;
    const row = { dataset: {}, remove() { list.children.splice(list.children.indexOf(row), 1); }, replaceWith(next) { list.children[list.children.indexOf(row)] = next; } };
    return row;
  };
  c.view.data = { session: { title: 'Title', scope_json: '{}' }, messages: [{ id: 1, seq: 1, role: 'user', status: 'done', parts_json: '[{"text":"Question"}]' }] };
  c.paintChatView(c.view); const original = list.children[0];
  c.paintChatView(c.view); assert.equal(builds, 1); assert.equal(list.children[0], original); assert.equal(list.scrollTop, 200);
  c.view.data.messages.push({ id: 2, role: 'assistant', status: 'done', parts_json: '[{"text":"Answer"}]' });
  c.paintChatView(c.view); assert.equal(builds, 2); assert.equal(list.children[0], original); assert.equal(list.scrollTop, 200);
});

test('chat navigation clears selected article and invalidates reader; composition Enter is ignored', async () => {
  const c = makeContext(); vm.runInContext(extract('renderChatView'), c); let sends = 0;
  c.sendChat = () => { sends++; }; await c.renderChatView();
  assert.equal(c.state.selectedId, null); assert.equal(c.state.readerEntry, null); assert.equal(c.readerToken, 1);
  const input = c.el('chat-input'); const enter = extra => ({ key: 'Enter', preventDefault() {}, ...extra });
  input.compositionstart(); input.onkeydown(enter()); assert.equal(sends, 0);
  input.compositionend(); input.onkeydown(enter({ isComposing: true })); input.onkeydown(enter({ keyCode: 229 })); input.onkeydown(enter({ shiftKey: true })); assert.equal(sends, 0);
  input.onkeydown(enter()); assert.equal(sends, 1);
});

test('mobile chat Back closes input before returning to digests; navigation remains five tabs', () => {
  const mobile = fs.readFileSync('ui/mobile.js', 'utf8');
  const start = mobile.indexOf('  function watchAndroidBack()');
  const end = mobile.indexOf('\n  }', start) + 4;
  let onBack, exits = 0, blurs = 0, homes = 0;
  const c = vm.createContext({
    document: { body: { dataset: { android: '1' } }, activeElement: { id: 'chat-input', blur() { blurs++; c.document.activeElement = null; } } },
    window: { __TAURI__: { app: { onBackButtonPress(fn) { onBack = fn; }, exit() { exits++; } } } },
    active: () => true, historyStack: [], bodyPage: () => 'digest', digestMode: 'chat',
    renderDigestHome() { homes++; }, history: { back() {} },
  });
  vm.runInContext(mobile.slice(start, end) + '\nwatchAndroidBack();', c);
  onBack(); assert.equal(blurs, 1); assert.equal(homes, 0); assert.equal(exits, 0);
  onBack(); assert.equal(homes, 1); assert.equal(exits, 0);
  assert.equal((fs.readFileSync('ui/index.html', 'utf8').match(/data-mpage-btn=/g) || []).length, 5);
});

test('i18n dictionaries have matching keys and all chat keys in both languages', () => {
  const c = vm.createContext({ window: {}, navigator: { language: 'en' }, document: { querySelectorAll: () => [] }, localStorage: { getItem: () => null } });
  vm.runInContext(fs.readFileSync('ui/i18n.js', 'utf8'), c);
  assert.equal(c.window.I18N.selfTest().ok, true);
  const dictionaries = c.window.I18N.DICTS;
  assert.equal(Object.keys(dictionaries.en).filter(k => k.startsWith('chat.')).length, 26);
  assert.deepEqual(Object.keys(dictionaries.en).sort(), Object.keys(dictionaries['zh-CN']).sort());
});

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function useRealPaint(c) {
  vm.runInContext(extract('paintChatView'), c);
  c.currentLocale = () => 'en';
  c.setText = (node, text) => { node.textContent = text; };
  for (const id of ['chat-error', 'chat-send', 'chat-delete', 'chat-input', 'chat-setup', 'chat-retry', 'chat-heading', 'chat-meta']) c.el(id).classList = { toggle() {} };
  Object.assign(c.el('chat-messages'), { children: [], scrollHeight: 1000, scrollTop: 200, clientHeight: 400 });
}
const persistedSession = { session: { id: 7, title: 'Bound session', provider: 'openai', model: 'test', scope_json: '{"date":"2026-10-03","scope_key":"tags:4","has_seed":true}' }, messages: [] };

test('deferred history load blocks input/send/retry and confirms the persisted scope after loading', async () => {
  const c = makeContext(); useRealPaint(c);
  vm.runInContext(extract('renderChatView') + '\n' + extract('refreshChatView'), c);
  const load = deferred(); const original = c.invoke;
  c.invoke = (command, args) => command === 'chat_session_get' ? load.promise : original(command, args);
  const opening = c.renderChatView(7);
  const view = c.chatView;
  assert.equal(c.el('chat-input').disabled, true); assert.equal(c.el('chat-send').disabled, true);
  assert.equal(c.el('chat-retry').disabled, true);
  assert.match(c.el('chat-meta').textContent, /chat.loading/); assert.match(c.el('chat-error').textContent, /chat.loading/);
  c.el('chat-input').value = 'draft'; view.lastText = 'retry';
  await c.sendChat(view); await c.sendChat(view, view.lastText);
  assert.equal(c.calls.length, 0); assert.equal(c.store.size, 0);
  load.resolve(persistedSession); await opening;
  assert.equal(c.el('chat-input').disabled, false); assert.equal(c.el('chat-send').disabled, false);
  assert.match(c.el('chat-meta').textContent, /2026-10-03 · tags:4/);
  c.confirmDialog = async args => { assert.match(args.body, /2026-10-03 · tags:4/); assert.doesNotMatch(args.body, /chat.noDigest/); return true; };
  await c.sendChat(view);
  assert.equal(c.calls.filter(x => x.command === 'chat_send').length, 1);
  assert.match([...c.store.values()][0], /2026-10-03 · tags:4/);
});

test('failed deferred history load remains blocked instead of authorizing unknown scope', async () => {
  const c = makeContext(); useRealPaint(c); vm.runInContext(extract('refreshChatView'), c);
  c.view.id = 7; const load = deferred(); c.invoke = () => load.promise;
  const loading = c.refreshChatView(c.view); load.reject(new Error('history unavailable')); await loading;
  assert.equal(c.el('chat-input').disabled, true); assert.equal(c.el('chat-send').disabled, true);
  await c.sendChat(c.view, 'retry'); assert.equal(c.store.size, 0); assert.equal(c.view.error, 'history unavailable');
});

test('deferred successful receipt preserves a newer draft even if completion arrived first', async () => {
  const c = makeContext(); const receipt = deferred(); const original = c.invoke;
  c.invoke = (command, args) => command === 'chat_send' ? receipt.promise : original(command, args);
  c.el('chat-input').value = 'submitted'; c.view.draft = 'submitted'; c.chatDrafts.set(null, 'submitted');
  const sending = c.sendChat(c.view); await new Promise(r => setImmediate(r));
  c.el('chat-input').value = 'new unsent draft'; c.view.draft = 'new unsent draft'; c.chatDrafts.set(null, c.view.draft);
  c.onChatTerminal({ payload: { sessionId: 7 } });
  receipt.resolve({ sessionId: 7 }); await sending;
  assert.equal(c.el('chat-input').value, 'new unsent draft'); assert.equal(c.view.draft, 'new unsent draft');
  assert.equal(c.chatDrafts.get(7), 'new unsent draft'); assert.equal(c.chatDrafts.has(null), false);
});

test('deferred receipt only clears an unchanged submitted snapshot', async () => {
  const c = makeContext(); const receipt = deferred(); const original = c.invoke;
  c.invoke = (command, args) => command === 'chat_send' ? receipt.promise : original(command, args);
  c.el('chat-input').value = 'submitted'; c.view.draft = 'submitted';
  const sending = c.sendChat(c.view); await new Promise(r => setImmediate(r));
  receipt.resolve({ sessionId: 7 }); await sending;
  assert.equal(c.el('chat-input').value, ''); assert.equal(c.view.draft, ''); assert.equal(c.chatDrafts.has(7), false);
});

test('locale change during deferred history load retranslates chat without replacing draft, route or scroll', async () => {
  const c = makeContext(); useRealPaint(c);
  vm.runInContext(extract('renderChatView') + '\n' + extract('refreshChatView') + '\n' + extract('rerenderChatI18n'), c);
  const load = deferred(); c.invoke = () => load.promise;
  const opening = c.renderChatView(7); const view = c.chatView;
  c.el('chat-input').value = 'draft'; view.draft = 'draft'; c.el('chat-messages').scrollTop = 200;
  const host = c.el('chat').innerHTML;
  for (const key of ['chat.newSession', 'chat.history', 'chat.delete', 'chat.needSetup', 'chat.errorRetry', 'chat.inputPlaceholder']) assert.match(host, new RegExp('data-i18n[^=]*="' + key + '"'));
  c.el('chat-history').options = [{ textContent: 'old' }];
  c.t = key => 'en:' + key; c.state.settings = { locale: 'en' };
  c.setLocale = () => {}; c.applyStaticI18n = () => {}; c.themeEditors = []; c.mountThemeEditor = () => ({}); c.rerenderLogsI18n = () => {};
  const start = source.indexOf('const SETTING_DROPDOWNS = [');
  const end = source.indexOf("  {\n    id: 'set-close-action'", start);
  await vm.runInContext(source.slice(start, end) + ']; SETTING_DROPDOWNS[0].after();', c);
  assert.equal(c.el('chat-send').textContent, 'en:chat.send'); assert.equal(c.el('chat-error').textContent, 'en:chat.loading');
  assert.equal(c.el('chat-history').options[0].textContent, 'en:chat.history');
  assert.equal(c.chatView, view); assert.equal(c.el('chat-input').value, 'draft'); assert.equal(c.el('chat-messages').scrollTop, 200);
  load.resolve(persistedSession); await opening;
  assert.equal(c.chatView, view); assert.equal(c.el('chat-input').value, 'draft'); assert.equal(c.el('chat-messages').scrollTop, 200);
});

for (const destination of ['article', 'another chat']) {
  test(`deferred delete cannot steal navigation to ${destination}`, async () => {
    const c = makeContext(); vm.runInContext(extract('renderChatView'), c);
    await c.renderChatView(7); const old = c.chatView;
    const deletion = deferred(); c.invoke = () => deletion.promise;
    c.chatDrafts.set(7, 'deleted draft');
    const deleting = c.el('chat-delete').onclick(); await new Promise(r => setImmediate(r));
    let renders = 0, refreshed;
    c.renderChatView = () => { renders++; }; c.refreshChatHistory = view => { refreshed = view; };
    if (destination === 'article') c.right.classList.remove('chat-active');
    else c.chatView = { id: 8 };
    deletion.resolve(); await deleting;
    assert.equal(renders, 0); assert.equal(c.chatDrafts.has(7), false);
    if (destination === 'another chat') assert.equal(refreshed, c.chatView);
    else assert.equal(refreshed, undefined);
    assert.equal(old.id, 7);
  });
}
