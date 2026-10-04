// 日报打开路径的互斥与令牌语义（审核 R5-P1 回归）。
// 场景 1：openDigestDate 清空 selectedId（s/u/l 不会改到看不见的文章）。
// 场景 2：历史请求在飞时打开文章，历史响应迟到必须被令牌丢弃（不得顶掉正文）。
const { test } = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');

const source = fs.readFileSync(
  path.join(__dirname, '..', '..', 'ui', 'app.js'),
  'utf8',
);
function extract(name) {
  const start = source.indexOf(`function ${name}(`);
  const end = source.indexOf('\n}', start) + 2;
  return (source.slice(start - 6, start) === 'async ' ? 'async ' : '') + source.slice(start, end);
}

function makeCtx() {
  const requests = [];
  let call = 0;
  const ctx = vm.createContext({
    state: { selectedId: 42, readerEntry: { id: 42 } },
    readerToken: 0,
    digestOpenDate: null,
    invoke: (_cmd, args) => {
      requests.push(args);
      return new Promise((resolve) => {
        // 挂起：测试手动放行
        ctx.__resolve = resolve;
      });
    },
    renderList() { ctx.__renderList = (ctx.__renderList || 0) + 1; },
    renderSidebar() { ctx.__renderSidebar = (ctx.__renderSidebar || 0) + 1; },
    renderDigestView(view) { ctx.__rendered = view; },
    digestScopeTags: () => [],
    setStatus() {},
    log() {},
  });
  ctx.__requests = requests;
  return ctx;
}

test('openDigestDate clears the selected entry (s/u/l cannot touch a hidden article)', async () => {
  const ctx = makeCtx();
  vm.runInContext(extract('openDigestDate'), ctx);
  const p = vm.runInContext('openDigestDate("2026-10-01", "tags:1")', ctx);
  assert.equal(ctx.state.selectedId, null, '打开日报必须清空条目选中');
  assert.equal(ctx.readerToken > 0, true, '必须推进 readerToken');
  assert.equal(ctx.__requests[0].scopeKey, 'tags:1', '历史行范围键直通');
  ctx.__resolve({ date: '2026-10-01', has_report: true });
  await p;
});

test('a late digest response must not replace an article opened meanwhile', async () => {
  const ctx = makeCtx();
  vm.runInContext(extract('openDigestDate'), ctx);
  const digestPromise = vm.runInContext('openDigestDate("2026-10-01")', ctx);
  const tokenAtDigest = vm.runInContext('readerToken', ctx);
  // 模拟「点了文章」：readerToken 被推进（openEntry/renderReader 的语义）
  vm.runInContext('readerToken++; state.selectedId = 7;', ctx);
  // 历史响应此时才回来
  ctx.__resolve({ date: '2026-10-01', has_report: true });
  await digestPromise;
  assert.equal(
    ctx.__rendered,
    undefined,
    '迟到的日报渲染必须被令牌校验丢弃',
  );
  assert.equal(ctx.state.selectedId, 7, '文章选中保持不动');
  assert.notEqual(vm.runInContext('readerToken', ctx), tokenAtDigest);
});
