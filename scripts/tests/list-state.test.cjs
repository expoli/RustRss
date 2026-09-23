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
function deferred() {
  let resolve;
  const promise = new Promise(r => { resolve = r; });
  return { promise, resolve };
}
test('footer performs zero writes when unchanged', () => {
  let writes = 0;
  const btn = {
    get disabled() { return false; }, set disabled(v) { writes++; },
    get textContent() { return 'more'; }, set textContent(v) { writes++; },
  };
  const ctx = vm.createContext({ sentinel: { querySelector: () => btn }, paging: { loading: false },
    sentinelLabel: () => 'more', t: x => x, setText: (node, text) => { if (node.textContent !== text) node.textContent = text; } });
  vm.runInContext(extract('refreshSentinelFooter'), ctx);
  vm.runInContext('refreshSentinelFooter()', ctx);
  assert.equal(writes, 0);
});
test('older total response cannot replace newer cache; resolved total refreshes footer', async () => {
  const pending = [deferred(), deferred()];
  let call = 0, footer = 0;
  const ctx = vm.createContext({ state: { view: { kind: 'tag', tagId: 1 } },
    unreadFilteredView: () => false, invoke: () => pending[call++].promise,
    renderListCount() {}, refreshSentinelFooter: () => { footer++; }, log() {}, el: () => ({ textContent: '' }) });
  vm.runInContext('let viewTotalCache = { key: null, n: null };' +
    extract('viewTotalKey') + extract('invalidateViewTotal') + extract('fetchViewTotal'), ctx);
  const old = vm.runInContext('fetchViewTotal()', ctx);
  vm.runInContext('invalidateViewTotal()', ctx);
  const fresh = vm.runInContext('fetchViewTotal()', ctx);
  pending[1].resolve(8); await fresh;
  pending[0].resolve(9); await old;
  assert.equal(vm.runInContext('viewTotalCache.n', ctx), 8);
  assert.equal(footer, 1);
});

test('old page cannot overwrite cursor after view reset', async () => {
  const pending = deferred();
  const ctx = vm.createContext({ invoke: () => pending.promise, PAGE_SIZE: 200,
    listArgs: () => ({}), paging: { generation: 1, cursor: null, exhausted: false } });
  if (source.includes('function listRequest(')) vm.runInContext(extract('listRequest'), ctx);
  vm.runInContext(extract('loadPage'), ctx);
  const old = vm.runInContext('loadPage(null)', ctx);
  ctx.paging.generation++;
  ctx.paging.cursor = { id: 22 };
  pending.resolve([{ id: 11, sortkey: 9, read: false }]);
  const rows = await old;
  assert.equal(ctx.paging.cursor.id, 22);
  assert.equal(rows, null);
});

test('folder list and totals carry the selected folder', () => {
  const ctx = vm.createContext({ state: { view: { kind: 'folder', folderId: 42 },
    feeds: [{ folder_id: 42, unread: 3 }, { folder_id: 9, unread: 8 }] },
    PAGE_SIZE: 200, unreadFilteredView: () => false });
  vm.runInContext(extract('listArgs') + extract('scopeUnread') + extract('viewTotalKey'), ctx);
  assert.equal(vm.runInContext('listArgs(null).folderId', ctx), 42);
  assert.equal(vm.runInContext('scopeUnread()', ctx), 3);
  assert.equal(vm.runInContext('viewTotalKey()', ctx), 'folder:42:a');
});

test('late article response cannot replace the more recently opened article', async () => {
  const requests = [deferred(), deferred()];
  let call = 0, displayed = null;
  const ctx = vm.createContext({ state: { selectedId: null }, paging: { generation: 1 },
    invoke: () => requests[call++].promise, renderReader: row => { displayed = row.id; },
    focusRow() {}, log() {} });
  vm.runInContext('let readerRequest = 0;' + extract('openEntry'), ctx);
  const old = vm.runInContext('openEntry(1)', ctx);
  const fresh = vm.runInContext('openEntry(2)', ctx);
  requests[1].resolve({ id: 2, read: true }); await fresh;
  requests[0].resolve({ id: 1, read: true }); await old;
  assert.equal(displayed, 2);
  assert.equal(ctx.state.selectedId, 2);
});

test('moving a feed refreshes the open folder membership', async () => {
  let resets = 0;
  const ctx = vm.createContext({ state: { view: { kind: 'folder', folderId: 1 } },
    invoke: async () => {}, refreshCounts: async () => {},
    loadEntries: async () => { resets++; }, setStatus() {} });
  vm.runInContext(extract('reassignFeed'), ctx);
  await vm.runInContext('reassignFeed(7, 2)', ctx);
  assert.equal(resets, 1);
});
