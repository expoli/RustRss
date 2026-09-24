// 抓取失败「码 → 文案 key → 双语字典」的契约测试。
//
// 为什么要有它：core（fetch.rs / network.rs）会以字符串码回失败，界面靠
// `fetchFailureMessage` 把码翻成 i18n key。漏一个码就静默落到「抓取失败，请稍后重试」，
// 用户看不出真正原因（代理配置坏了 vs 网络断了）——本测试把这件事变成会红。
//
// 新增码时：先在 core 里落地码 → 加进 EXPECTED_CODES → 在 ui/app.js 的映射表与
// ui/i18n.js 两份字典里各加一条。
const assert = require('node:assert/strict');
const test = require('node:test');
const fs = require('node:fs');
const path = require('node:path');

const root = path.resolve(__dirname, '..', '..');
const app = fs.readFileSync(path.join(root, 'ui', 'app.js'), 'utf8');
const i18n = fs.readFileSync(path.join(root, 'ui', 'i18n.js'), 'utf8');

// core 能发给界面的失败码（与 fetch.rs / network.rs 的字面量同名）
const EXPECTED_CODES = [
  'retry_deferred', 'timeout', 'connection_error', 'network_error', 'redirect_error',
  'invalid_url', 'too_large', 'body_error', 'parse_error', 'no_feed_link',
  'unexpected_response',
  'proxy_client_lock', 'proxy_client_setup', 'proxy_invalid_url', 'proxy_invalid_config',
  'proxy_credentials_not_supported',
];

function mappingSource() {
  const start = app.indexOf('function fetchFailureMessage');
  const end = app.indexOf('function folderHead');
  assert.ok(start > 0 && end > start, '未找到 fetchFailureMessage');
  return app.slice(start, end);
}

test('every fetch failure code the core can emit has a mapping', () => {
  const fn = mappingSource();
  for (const code of EXPECTED_CODES) {
    assert.ok(fn.includes(`${code}:`), `未映射的抓取失败码: ${code}`);
  }
});

test('every mapped key exists in both language dictionaries', () => {
  const fn = mappingSource();
  const keys = [...fn.matchAll(/[a-z_]+:\s*'([A-Za-z]+)'/g)].map((m) => m[1]);
  assert.ok(keys.length >= EXPECTED_CODES.length, `映射数量可疑: ${keys.length}`);
  for (const key of keys) {
    const needle = `'fetchError.${key}'`;
    const hits = i18n.split(needle).length - 1;
    assert.equal(hits, 2, `两份字典都应含 ${needle}（实际 ${hits} 处）`);
  }
});
