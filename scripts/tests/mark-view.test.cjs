const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

const source = fs.readFileSync('ui/app.js', 'utf8');
const markAll = source.slice(source.indexOf('async function markAll(read)'), source.indexOf('let currentAiTask'));

for (const view of [
  { kind: 'all' }, { kind: 'unread' }, { kind: 'starred' }, { kind: 'later' },
  { kind: 'feed', feedId: 7 }, { kind: 'tag', tagId: 9 }, { kind: 'folder', folderId: 3 },
  { kind: 'search' },
]) {
  test(`batch marking sends explicit ${view.kind} scope`, async () => {
    const calls = [];
    const ctx = vm.createContext({
      state: { view, feedId: view.feedId ?? null, query: 'Rust', entries: [], readSessionIds: new Set() },
      invoke: async (cmd, args) => { calls.push({ cmd, args }); return 0; },
      t: () => '', setStatus() {}, log() {}, loadAll: async () => {},
      el: () => ({ classList: { add() {} } }),
    });
    vm.runInContext(markAll, ctx);
    await vm.runInContext('markAll(true)', ctx);
    const scope = JSON.parse(JSON.stringify(calls[0].args.scope));
    assert.equal(scope.kind, view.kind);
    if (view.kind === 'feed') assert.equal(scope.id, 7);
    if (view.kind === 'tag') assert.equal(scope.id, 9);
    if (view.kind === 'folder') assert.equal(scope.id, 3);
    if (view.kind === 'search') assert.equal(scope.query, 'Rust');
  });
}
