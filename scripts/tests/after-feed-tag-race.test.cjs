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
  vm.runInContext(extract('afterFeedTagChange'), ctx);
  const first = vm.runInContext('afterFeedTagChange()', ctx); // 第一次调用（旧标签归属）
  const second = vm.runInContext('afterFeedTagChange()', ctx); // 第二次（源标签提交完成后的新回读）
  // 第二次调用的 get_entry（deferreds[1]）先 resolve 新标签
  deferreds[1].resolve({ id: 7, tags: [{ id: 9, name: '新标签', source: 'feed' }] });
  await second;
  // 第一次调用的 get_entry（deferreds[0]）后 resolve 旧标签（迟到的过期回读）
  deferreds[0].resolve({ id: 7, tags: [{ id: 8, name: '旧标签', source: 'feed' }] });
  await first;
  await new Promise((r) => setTimeout(r, 10));
  // 期望：迟到者被 readerToken/身份守卫丢弃——但两次调用间 readerToken 未变、
  // state.readerEntry 也未变（同一个对象），守卫不成立 → __applied 是旧标签 = 竞态实锤
  console.log('applied:', JSON.stringify(ctx.__applied));
  assert.deepEqual(ctx.__applied.tags.map((t) => t.name), ['新标签'],
    '迟到回读不得覆盖新标签（若本断言失败 = 竞态确认）');
});
