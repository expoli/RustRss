const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');

const componentsContext = vm.createContext({});
vm.runInContext(fs.readFileSync('ui/components.js', 'utf8'), componentsContext);

test('list thumbnail markup escapes remote URLs and suppresses referrer', () => {
  const components = componentsContext.RustRssComponents;
  const escaped = components.thumbnailImage('https://img.example/a?x=1&y="z"', value =>
    value.replaceAll('&', '&amp;').replaceAll('"', '&quot;'));
  assert.match(escaped, /loading="lazy"/);
  assert.match(escaped, /decoding="async"/);
  assert.match(escaped, /referrerpolicy="no-referrer"/);
  assert.match(escaped, /src="https:\/\/img\.example\/a\?x=1&amp;y=&quot;z&quot;"/);
  assert.equal(components.thumbnailImage(null, value => value), '');
  assert.match(components.entryContent({ title: 'Article', meta: '', summary: '', thumbnail: escaped }),
    /^<img class="entry-thumbnail"/);
  assert.match(fs.readFileSync('ui/app.js', 'utf8'), /thumbnailImage\(e\.thumbnail_url, escapeHtml\)/);
});

test('theme thumbnail preference toggles the existing list image without replacing it', () => {
  const themeContext = vm.createContext({});
  vm.runInContext(fs.readFileSync('ui/theme.js', 'utf8'), themeContext);
  const root = { style: { setProperty() {} }, dataset: {} };
  const renderer = themeContext.RustRssTheme.createRenderer(root, { media: null });
  const values = () => ({
    colors: Object.fromEntries(Object.keys(themeContext.RustRssTheme.colorVars).map(key => [key, '#123456'])),
    typography: { ui_family: ['sans-serif'], read_family: ['serif'], mono_family: ['monospace'], ui_size: 14, read_size: 18, mono_size: 13, line_height: 1.6 },
    list: { density: 'comfortable', summary_lines: 2, thumbnail: true },
    reader: { width: 680, paragraph_gap: 1, layout: 'three_column' },
    chrome: { radius: 8, sidebar_width: 220, list_width: 340 },
  });
  const snapshot = { config: { revision: 1, mode: 'light' }, config_hash: 'fixture', light: values(), dark: values() };
  const thumbnail = { isConnected: true, className: 'entry-thumbnail' };
  const row = { querySelector: () => thumbnail };
  renderer.apply(snapshot);
  assert.equal(root.dataset.thumbnails, 'true');
  snapshot.light.list.thumbnail = false;
  renderer.apply(snapshot);
  assert.equal(root.dataset.thumbnails, 'false');
  assert.equal(row.querySelector('.entry-thumbnail'), thumbnail);
  assert.match(fs.readFileSync('ui/style.css', 'utf8'), /html\[data-thumbnails="false"\] \.entry-thumbnail \{ display: none; \}/);
});
