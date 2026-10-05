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
function picker(mode = 'entry') {
  const tags = [{ id: 1, name: 'inherited', unread: 2 }, { id: 2, name: 'both', unread: 2 }, { id: 3, name: 'manual', unread: 1 }];
  const list = { innerHTML: '', querySelector: () => null };
  const ctx = vm.createContext({ state: { tags }, tagPickerMode: mode, tagPickerFeedTagIds: [1, 2],
    tagPickerEntryId: 7, tagPickerRows: [], tagPickerIndex: 0, el: () => list,
    entryTags: () => tags.map((t, i) => ({ ...t, source: ['feed', 'both', 'manual'][i] })),
    escapeHtml: s => String(s).replaceAll('"', '&quot;'), t: key => key, tagColorStyle: () => '', log() {} });
  vm.runInContext(extract('renderTagPicker'), ctx);
  vm.runInContext("renderTagPicker('')", ctx);
  return list.innerHTML;
}
test('entry picker disables inherited and both checkboxes, labels provenance and explains edits', () => {
  const html = picker();
  const rows = html.match(/<li.*?<\/li>/g);
  for (let i = 0; i < 2; i++) {
    assert.match(rows[i], /aria-disabled="true"/);
    assert.match(rows[i], /title="tags.inheritedHint"/);
    assert.match(rows[i], /type="checkbox".* checked disabled/);
  }
  assert.match(rows[0], /tags.inherited</);
  assert.match(rows[1], /tags.both</);
  assert.doesNotMatch(rows[2], /disabled|tags.inherited/);
  assert.match(rows[2], /type="checkbox".* checked/);
});
test('feed picker keeps the same tags editable', () => assert.doesNotMatch(picker('feed'), /disabled|tags.inherited|tags.both/));
for (const provenance of ['feed', 'both', 'manual']) {
  test(`toggleTagOnEntry ${provenance} blocks inherited writes but permits manual removal`, async () => {
    const calls = [];
    const ctx = vm.createContext({ entryTags: () => [{ id: 1, source: provenance }],
      invoke: async cmd => calls.push(cmd), afterTagChange: async () => {}, setStatus() {}, log() {} });
    vm.runInContext(extract('toggleTagOnEntry'), ctx);
    await vm.runInContext('toggleTagOnEntry(7, {id: 1})', ctx);
    assert.deepEqual(calls, provenance === 'manual' ? ['unassign_tags'] : []);
  });
}
test('feed write refreshes data even when picker is closed during commit', async () => {
  const calls = [];
  const ctx = vm.createContext({ tagPickerRows: [{ kind: 'tag', tag: { id: 9 } }], tagPickerIndex: 0,
    tagPickerMode: 'feed', tagPickerFeedId: 3, tagPickerFeedTagIds: [], pickerSession: 1,
    invoke: async (cmd, args) => { calls.push([cmd, args]); ctx.pickerSession++; },
    afterFeedTagChange: async () => calls.push(['refresh']), renderTagPicker: () => assert.fail('closed picker rendered') });
  vm.runInContext(extract('confirmTagPickerRow'), ctx);
  await vm.runInContext('confirmTagPickerRow()', ctx);
  assert.equal(calls[0][0], 'set_feed_tags');
  assert.deepEqual(JSON.parse(JSON.stringify(calls[0][1])), { feedId: 3, tagIds: [9] });
  assert.deepEqual(calls[1], ['refresh']);
  assert.deepEqual(ctx.tagPickerFeedTagIds, []);
});
for (const changedReader of [false, true]) {
  test(`afterFeedTagChange refreshes list/chips/counts, guards stale reader (${changedReader})`, async () => {
    const calls = [];
    let resolve;
    const pending = new Promise(r => { resolve = r; });
    const shown = { id: 7, tags: [] };
    const ctx = vm.createContext({ state: { readerEntry: shown, view: { kind: 'tag' } }, readerToken: 1, tagGeneration: 1,
      invoke: (cmd, args) => { calls.push([cmd, args]); return pending; },
      refreshTagCache: async () => calls.push(['cache']), refreshCounts: async () => calls.push(['counts']),
      loadEntries: async args => calls.push(['list', args]), setEntryTags: (id, tags) => calls.push(['tags', id, tags]),
      patchReaderTags: () => calls.push(['chips']) });
    vm.runInContext(extract('afterFeedTagChange'), ctx);
    const task = vm.runInContext('afterFeedTagChange()', ctx);
    if (changedReader) { ctx.state.readerEntry = { id: 8 }; ctx.readerToken++; ctx.tagGeneration++; }
    resolve({ id: 7, tags: [{ id: 1, source: 'feed' }] });
    await task;
    assert.ok(calls.some(c => c[0] === 'cache'));
    assert.ok(calls.some(c => c[0] === 'counts'));
    assert.deepEqual(JSON.parse(JSON.stringify(calls.find(c => c[0] === 'list')[1])), { reader: false, reuseRows: true });
    assert.equal(calls.some(c => c[0] === 'chips'), !changedReader);
  });
}
test('feed-tag reread uses keyed existing list nodes and patches only chips', () => {
  let clears = 0, patched = [];
  const old = { dataset: { id: '1' } }, removed = { dataset: { id: '2' } }, fresh = { dataset: { id: '3' } };
  const list = { children: [old, removed], set innerHTML(v) { clears++; } };
  const ctx = vm.createContext({ state: { view: { kind: 'tag' }, entries: [{ id: 1 }, { id: 3 }], selectedId: 1 },
    document: { body: { dataset: {} } }, window: {}, performance: { now: () => 0 }, el: id => id === 'entries' ? list : {},
    viewTitle: () => '', renderListCount() {}, renderUnreadOnlyButton() {}, fetchViewTotal: async () => {},
    buildEntryRow: e => { assert.equal(e.id, 3); return fresh; }, reconcileChildren: (container, rows) => { container.children = rows; },
    patchRowTags: id => patched.push(id), installSentinel() {}, focusRow: (id, opts) => assert.equal(opts.follow, false), log() {} });
  vm.runInContext(extract('renderList'), ctx);
  vm.runInContext('renderList({reuseRows: true})', ctx);
  assert.equal(clears, 0);
  assert.equal(list.children[0], old);
  assert.equal(list.children[1], fresh);
  assert.deepEqual(patched, [1, 3]);
});
test('feed tags do not load an article list over the digest or chat view', async () => {
  for (const kind of ['digest', 'chat']) {
    const ctx = vm.createContext({ state: { view: { kind }, readerEntry: null }, readerToken: 1, tagGeneration: 1,
      refreshTagCache: async () => {}, refreshCounts: async () => {}, loadEntries: () => assert.fail('article list requested') });
    vm.runInContext(extract('afterFeedTagChange'), ctx);
    await vm.runInContext('afterFeedTagChange()', ctx);
  }
});
