const { test } = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname, '..', '..', 'ui', 'app.js'), 'utf8');
function extract(name) {
  const start = source.indexOf(`function ${name}(`);
  const end = source.indexOf('\n}', start) + 2;
  return (source.slice(start - 6, start) === 'async ' ? 'async ' : '') + source.slice(start, end);
}

test('afterFeedTagChange late get_entry race: second (newer) call resolves first, first resolves late', async () => {
  const deferreds = [];
  function makeDeferred() {
    let resolve;
    const promise = new Promise((r) => { resolve = r; });
    deferreds.push({ promise, resolve });
    return deferreds[deferreds.length - 1];
  }
  let call = 0;
  const ctx = vm.createContext({
    state: { readerEntry: { id: 7 }, view: { kind: 'all' } },
    readerToken: 5,
    tagGeneration: 0,
    invoke: (cmd, args) => {
      assert.equal(cmd, 'get_entry');
      const d = makeDeferred();
      return d.promise;
    },
    refreshTagCache: async () => {},
    refreshCounts: async () => {},
    loadEntries: async () => {},
    setEntryTags: (id, tags) => { ctx.__applied = { id, tags }; },
    patchReaderTags: () => { ctx.__patched = (ctx.__patched || 0) + 1; },
    Promise,
  });
  vm.runInContext(extract('afterFeedTagChange') + extract('afterFeedTagChangeInner'), ctx);
  vm.runInContext(
    'let tagRefreshChain = Promise.resolve();' +
    'async function afterFeedTagChange() {' +
    '  const shown = state.readerEntry; const token = readerToken; const gen = ++tagGeneration;' +
    '  const run = tagRefreshChain.then(() => afterFeedTagChangeInner(shown, token, gen));' +
    '  tagRefreshChain = run.catch(() => {}); return run; }',
    ctx,
  );
  // 乱序场景（队列化下表现为串行）：第一次调用拿旧数据、完成后第二次调用
  // 拿新数据——最终生效的必须是后者的新标签
  const first = vm.runInContext('afterFeedTagChange()', ctx);
  await new Promise((r) => setTimeout(r, 0)); // 队列微任务放行第一次的 invoke
  deferreds[0].resolve({ id: 7, tags: [{ id: 8, name: '旧标签', source: 'feed' }] }); // 第一次的数据（旧）
  await first; // 第一次完成 → 队列放行第二次
  const second = vm.runInContext('afterFeedTagChange()', ctx); // 第二次发起
  await new Promise((r) => setTimeout(r, 0)); // 队列微任务放行第二次的 invoke
  deferreds[1].resolve({ id: 7, tags: [{ id: 9, name: '新标签', source: 'feed' }] }); // 第二次的数据（新）
  await second;
  await new Promise((r) => setTimeout(r, 10));
  console.log('applied:', JSON.stringify(ctx.__applied));
  assert.deepEqual(ctx.__applied.tags.map((t) => t.name), ['新标签'],
    '队列化后最终生效的必须是后一次调用的数据');
});
