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
      addEventListener(event, fn) { this[event] = fn; } });
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
    refreshChatView: async () => {}, refreshChatHistory: async () => {},
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
  context.view = { id: null, pending: false };
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
  c.view.date = null; c.view.scopeKey = null;
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
  c.view.pending = false; c.view.running = true; c.view.id = 7;
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
  assert.equal(Object.keys(dictionaries.en).filter(k => k.startsWith('chat.')).length, 25);
  assert.deepEqual(Object.keys(dictionaries.en).sort(), Object.keys(dictionaries['zh-CN']).sort());
});
