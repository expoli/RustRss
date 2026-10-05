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

test('queued feed-tag refreshes: second invoke waits for first, fresher data wins', async () => {
  const getDeferreds = [];   // get_entry 的 deferred（按调用序）
  const cacheDeferreds = []; // refreshTagCache 的 deferred（按调用序）
  const applied = [];
  const ctx = vm.createContext({
    state: { readerEntry: { id: 7 }, view: { kind: 'all' }, tags: [{ id: 0, name: '旧未读' }] },
    readerToken: 5,
    tagGeneration: 5,
    invoke: (cmd, args) => {
      assert.equal(cmd, 'get_entry');
      let resolve;
      const promise = new Promise((r) => { resolve = r; });
      getDeferreds.push({ promise, resolve, args });
      return promise;
    },
    refreshTagCache: async () => {
      let resolve;
      const promise = new Promise((r) => { resolve = r; });
      cacheDeferreds.push({ resolve, tags: null });
      const tags = await promise;
      ctx.state.tags = tags; // 真实函数行为：写回 state
      return tags;
    },
    refreshCounts: async () => { ctx.__counts = (ctx.__counts || 0) + 1; },
    loadEntries: async () => {},
    setEntryTags: (id, tags) => { applied.push({ id, tags }); },
    patchReaderTags: () => {},
    Promise,
  });
  // 只注入 tagRefreshChain 变量；afterFeedTagChange/Inner 均为生产源码抽取
  vm.runInContext(
    'let tagRefreshChain = Promise.resolve();' + extract('afterFeedTagChange') + extract('afterFeedTagChangeInner'),
    ctx,
  );
  // 同时发起两次调用
  const first = vm.runInContext('afterFeedTagChange()', ctx);
  const second = vm.runInContext('afterFeedTagChange()', ctx);
  await new Promise((r) => setTimeout(r, 0)); // 队列微任务放行第一次的 invoke
  // 队列语义：second 的 get_entry 未启动（仍在排队）
  assert.equal(getDeferreds.length, 1, 'second invoke 未启动（排队中）');
  // 释放 first：get_entry 旧数据 + cache 旧未读数
  getDeferreds[0].resolve({ id: 7, tags: [{ id: 8, name: '旧标签', source: 'feed' }] });
  cacheDeferreds[0].resolve([{ id: 0, name: '旧未读' }]);
  await first;
  await new Promise((r) => setTimeout(r, 0)); // 队列放行第二次的 invoke
  // first 完成后 second 的 invoke 启动
  assert.equal(getDeferreds.length, 2, 'first 完成后 second 的 invoke 启动');
  // 释放 second：get_entry 新数据 + cache 新未读数
  getDeferreds[1].resolve({ id: 7, tags: [{ id: 9, name: '新标签', source: 'feed' }] });
  cacheDeferreds[1].resolve([{ id: 9, name: '新未读' }]);
  await second;
  await new Promise((r) => setTimeout(r, 10));
  // 最终生效的必须是后一次调用（fresher）的数据
  assert.deepEqual(applied[applied.length - 1].tags.map((t) => t.name), ['新标签'],
    '迟到/排队回读后最终生效的必须是新数据');
  assert.deepEqual(ctx.state.tags, [{ id: 9, name: '新未读' }],
    '缓存回读最终生效的必须是新未读数');
});

test('a rejected branch must not release the queue while other re-reads are pending', async () => {
  const getDeferreds = [];
  const cacheDeferreds = [];
  let countsCalls = 0;
  const applied = [];
  const ctx = vm.createContext({
    state: { readerEntry: { id: 7 }, view: { kind: 'all' }, tags: [{ id: 0, name: '旧未读' }] },
    readerToken: 5,
    tagGeneration: 5,
    invoke: (cmd, args) => {
      assert.equal(cmd, 'get_entry');
      let resolve;
      const promise = new Promise((r) => { resolve = r; });
      getDeferreds.push({ promise, resolve, args });
      return promise;
    },
    refreshTagCache: async () => {
      let resolve;
      const promise = new Promise((r) => { resolve = r; });
      cacheDeferreds.push({ resolve, tags: null });
      const tags = await promise;
      ctx.state.tags = tags;
      return tags;
    },
    // 第一次立即 reject、第二次正常——验证失败不提前放行队列
    refreshCounts: async () => {
      countsCalls += 1;
      if (countsCalls === 1) throw new Error('sidebar_data boom');
    },
    loadEntries: async () => {},
    setEntryTags: (id, tags) => { applied.push({ id, tags }); },
    patchReaderTags: () => {},
    Promise,
  });
  vm.runInContext(
    'let tagRefreshChain = Promise.resolve();' + extract('afterFeedTagChange') + extract('afterFeedTagChangeInner'),
    ctx,
  );
  // first：counts 立即 reject，get_entry/cache 挂起（allSettled 等全部分支
  // settle——失败分支不会提前放行队列，second 保持排队）
  const first = vm.runInContext('afterFeedTagChange()', ctx);
  const second = vm.runInContext('afterFeedTagChange()', ctx);
  await new Promise((r) => setTimeout(r, 0));
  assert.equal(getDeferreds.length, 1, 'first 失败未提前放行队列（second 仍排队）');
  // 释放 first 的全部挂起分支（cache + get_entry）→ first 以失败完成、队列放行
  getDeferreds[0].resolve({ id: 7, tags: [{ id: 8, name: '旧标签', source: 'feed' }] });
  cacheDeferreds[0].resolve([{ id: 0, name: '旧未读' }]);
  await first.catch(() => {});
  await new Promise((r) => setTimeout(r, 0)); // 队列放行 second 的 invoke
  // second 正常完成回读新值
  getDeferreds[1].resolve({ id: 7, tags: [{ id: 9, name: '新标签', source: 'feed' }] });
  cacheDeferreds[1].resolve([{ id: 9, name: '新未读' }]);
  await second;
  await new Promise((r) => setTimeout(r, 10));
  // 失败不提前放行 → second 的新值是最终写入（无 first 残留覆盖）
  assert.deepEqual(ctx.state.tags, [{ id: 9, name: '新未读' }],
    'second 的新未读数保持最终生效（失败分支不放行队列）');
  assert.deepEqual(applied[applied.length - 1].tags.map((t) => t.name), ['新标签'],
    'second 的回读保持最终生效');
});
