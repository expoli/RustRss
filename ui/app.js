// RustRss 界面逻辑。
//
// 整个文件包在 IIFE 里：避免顶层声明变成 window 属性而与 i18n.js 的同名函数冲突
// （WebKit 下这种冲突是解析期 SyntaxError，整份脚本都不会执行）。
(function () {

const el = (id) => document.getElementById(id);

// 最早的诊断：先确认脚本真的被执行了。
// 这样「脚本没跑」与「跑了一半报错」能被日志区分开——否则只能靠猜。
try {
  window.__TAURI__ && window.__TAURI__.core
    ? window.__TAURI__.core.invoke('ui_log', { line: 'app.js start' })
    : console.error('IPC 不可用');
} catch (err) {
  console.error('ui_log 不可用', err);
}

// i18n 缺失时不能静默：把问题报出来，界面退化为显示 key（不空白）
const I18N_API = window.I18N || null;
if (!I18N_API) {
  try {
    window.__TAURI__?.core?.invoke('ui_log', { line: 'FATAL: window.I18N 未定义（i18n.js 未加载？）' });
  } catch {}
  console.error('window.I18N 未定义');
}
const t = I18N_API ? I18N_API.t : (key) => key;
const applyStaticI18n = I18N_API ? I18N_API.applyStaticI18n : () => {};
const i18nSelfTest = I18N_API
  ? I18N_API.selfTest
  : () => ({ ok: false, keys: 0, problems: ['i18n.js 未加载'] });
const setLocale = I18N_API ? I18N_API.setLocale : () => 'zh-CN';
const currentLocale = () => (I18N_API ? I18N_API.locale() : 'zh-CN');

async function invoke(cmd, args = {}) {
  if (!window.__TAURI__ || !window.__TAURI__.core) {
    throw new Error('IPC 不可用（不在 Tauri 中运行？）');
  }
  try {
    return await window.__TAURI__.core.invoke(cmd, args);
  } catch (e) {
    throw new Error(typeof e === 'string' ? e : (e && e.message ? e.message : String(e)));
  }
}

/** 诊断输出：打到应用 stdout，便于无人值守时核对界面状态（不依赖肉眼看屏幕） */
function log(line) {
  console.log('[ui]', line);
  invoke('ui_log', { line }).catch(() => {});
}

const state = {
  db: null,
  feeds: [],
  entries: [],
  view: { kind: 'unread' },
  feedId: null,
  selectedId: null,
  query: '',
  // 权威值在 Rust 侧（get_ui_settings），这里只是启动前的占位
  settings: { mark_read_on_navigate: true },
  ai: null,
  mcp: null,
};

const VIEWS = [
  { kind: 'unread', key: 'list.unread', icon: '●' },
  { kind: 'starred', key: 'list.starred', icon: '★' },
  { kind: 'all', key: 'list.all', icon: '≡' },
];

// ---------------------------------------------------------------- 正文清洗

const ALLOWED_TAGS = {
  p: [], br: [], hr: [], h1: [], h2: [], h3: [], h4: [], h5: [], h6: [],
  ul: [], ol: [], li: [], blockquote: [], pre: [], code: [],
  table: [], thead: [], tbody: [], tfoot: [], tr: [], th: [], td: [],
  figure: [], figcaption: [], div: [], span: [],
  strong: [], em: [], b: [], i: [], u: [], s: [], del: [], ins: [], sup: [], sub: [], mark: [],
  img: ['src', 'alt', 'title', 'width', 'height'],
  a: ['href', 'title'],
};

const DROP_ENTIRELY = new Set([
  'script', 'style', 'iframe', 'object', 'embed', 'form', 'input', 'button',
  'textarea', 'select', 'link', 'meta', 'base', 'svg', 'math', 'video', 'audio', 'source',
]);

/** 危险协议：这些一律不允许出现在 href/src 上 */
const DANGEROUS_SCHEME = /^\s*(javascript|vbscript|file|blob):/i;

/**
 * 白名单清洗：不在名单里的标签只保留文字内容（unwrap），危险标签整段丢弃。
 * 同时把相对路径的图片解析成绝对地址（否则图片全是裂的）。
 *
 * 注意：清洗只对 `body` 的子树做，**不能把 body 自己当普通元素评估** ——
 * body 不在白名单里，会被当作「未知标签」unwrap 掉，导致后续 doc.body 为空。
 * （这个 bug 是启动时通过 ui_log 上报报错拓出来的，不是看界面看出来的。）
 */
function sanitize(html, baseUrl) {
  let doc;
  try {
    doc = new DOMParser().parseFromString(html, 'text/html');
  } catch {
    return escapeHtml(html);
  }
  const root = doc && (doc.body || doc.documentElement);
  if (!root) return escapeHtml(html);

  const cleanElement = (node) => {
    const tag = node.tagName.toLowerCase();
    if (DROP_ENTIRELY.has(tag)) {
      node.remove();
      return;
    }
    if (!(tag in ALLOWED_TAGS)) {
      const parent = node.parentNode;
      if (!parent) return;
      while (node.firstChild) parent.insertBefore(node.firstChild, node);
      node.remove();
      return;
    }

    const allowed = ALLOWED_TAGS[tag];
    for (const attr of Array.from(node.attributes)) {
      const name = attr.name.toLowerCase();
      if (!allowed.includes(name)) {
        node.removeAttribute(attr.name);
        continue;
      }
      const value = attr.value.trim();
      // 只拦危险协议，**不要**在这里判相对地址：相对地址是正常的，
      // 会在下面被解析成绝对地址。（先前就是在这里把相对图片直接删了，
      // 导致 feed 里的图片全不显示 —— 自检把它拓了出来。）
      if ((name === 'href' || name === 'src') && DANGEROUS_SCHEME.test(value)) {
        node.removeAttribute(attr.name);
        continue;
      }
      if (name === 'src' && /^data:/i.test(value) && !/^data:image\//i.test(value)) {
        node.removeAttribute('src');
      }
    }

    if (tag === 'a') {
      node.setAttribute('rel', 'noopener noreferrer');
      const href = node.getAttribute('href');
      if (!href) {
        node.remove();
        return;
      }
      // 相对链接也要解析成绝对地址，否则交给系统浏览器时无法打开
      if (!/^[#]/.test(href) && !/^data:/i.test(href)) {
        try {
          node.setAttribute('href', new URL(href, baseUrl || location.href).toString());
        } catch {
          node.removeAttribute('href');
          node.remove();
        }
      }
    }
    if (tag === 'img') {
      const src = node.getAttribute('src');
      if (!src) return;
      if (!/^data:/i.test(src)) {
        try {
          node.setAttribute('src', new URL(src, baseUrl || location.href).toString());
        } catch {
          node.removeAttribute('src');
        }
      }
      node.setAttribute('loading', 'lazy');
      node.setAttribute('referrerpolicy', 'no-referrer');
    }
  };

  // 后序遍历：先处理子树再评估自身，这样 unwrap 不会丢掉已处理好的内容
  const walkChildren = (node) => {
    for (const child of Array.from(node.childNodes)) {
      if (child.nodeType === 1) {
        walkChildren(child);
        cleanElement(child);
      } else if (child.nodeType === 8) {
        child.remove(); // 注释一并去掉
      }
    }
  };

  walkChildren(root);
  return root.innerHTML;
}

// ---------------------------------------------------------------- 渲染

function fmtTime(ts) {
  if (!ts) return '';
  const d = new Date(ts * 1000);
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  const loc = currentLocale();
  return sameDay
    ? d.toLocaleTimeString(loc, { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString(loc, { year: 'numeric', month: '2-digit', day: '2-digit' });
}

function setStatus(text, isError = false) {
  const node = el('status');
  node.textContent = text || '';
  node.classList.toggle('error', isError);
}

function renderSidebar() {
  const views = el('views');
  views.innerHTML = '';
  const counts = {
    unread: state.db ? state.db.unread : 0,
    starred: state.db ? state.db.starred : 0,
    all: state.db ? state.db.entries : 0,
  };
  for (const v of VIEWS) {
    const li = document.createElement('li');
    li.className = state.view.kind === v.kind ? 'active' : '';
    li.innerHTML = `<span class="icon">${v.icon}</span><span>${t(v.key)}</span><span class="count">${counts[v.kind]}</span>`;
    li.onclick = () => setView({ kind: v.kind });
    views.appendChild(li);
  }

  const feeds = el('feeds');
  feeds.innerHTML = '';
  for (const f of state.feeds) {
    const li = document.createElement('li');
    const failed = f.last_status && f.last_status !== 'ok' && f.last_status !== 'not_modified';
    li.className = state.view.kind === 'feed' && state.feedId === f.id ? 'active' : '';
    li.title = failed
      ? t('sidebar.feedTooltipFailed', {
          status: f.last_status,
          error: f.last_error || '',
        })
      : t('sidebar.feedTooltipOk', { url: f.url });
    li.innerHTML = `<span class="name">${escapeHtml(f.title)}</span>${failed ? '<span class="dot">●</span>' : ''}<span class="count">${f.unread}</span>`;
    li.onclick = () => setView({ kind: 'feed', feedId: f.id });
    li.ondblclick = () => refreshOne(f.id);
    feeds.appendChild(li);
  }
  el('feeds-meta').textContent = t('sidebar.feedCount', { n: state.feeds.length });
  if (state.db) {
    const info = el('db-info');
    info.textContent = `${state.db.entries} 篇 · ${state.db.dbPath}`;
    info.title = state.db.dbPath;
  }
}

function escapeHtml(text) {
  return String(text ?? '').replace(/[&<>"']/g, (c) => ({
    '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
  }[c]));
}

function viewTitle() {
  if (state.view.kind === 'search') return t('list.searchTitle', { q: state.query });
  if (state.view.kind === 'feed') {
    const feed = state.feeds.find((f) => f.id === state.feedId);
    return feed ? feed.title : t('list.feedFallback');
  }
  const view = VIEWS.find((v) => v.kind === state.view.kind);
  return view ? t(view.key) : t('list.all');
}

function renderList() {
  el('list-title').textContent = viewTitle();
  el('list-count').textContent = state.entries.length ? t('list.count', { n: state.entries.length }) : '';

  const list = el('entries');
  list.innerHTML = '';
  if (!state.entries.length) {
    const li = document.createElement('li');
    li.className = 'dim';
    li.style.cursor = 'default';
    li.textContent = state.view.kind === 'unread' ? t('list.emptyUnread') : t('list.empty');
    list.appendChild(li);
    return;
  }

  for (const e of state.entries) {
    const li = document.createElement('li');
    li.className = `${e.id === state.selectedId ? 'active' : ''} ${e.read ? 'read' : ''}`;
    li.dataset.id = String(e.id);
    const star = e.starred ? '<span class="star">★</span>' : '';
    li.innerHTML = `
      <span class="title">${escapeHtml(e.title)}</span>
      <span class="meta"><span>${escapeHtml(e.feed_title)}</span><span>${fmtTime(e.published_at)}</span>${star}</span>
      ${e.summary ? `<span class="summary">${escapeHtml(e.summary)}</span>` : ''}`;
    li.onclick = () => openEntry(e.id, { markRead: true });
    list.appendChild(li);
  }
  focusRow(state.selectedId, { follow: true });
}

/**
 * 只更新「哪一行是当前行」 —— 不重建列表。
 *
 * 这件事必须和渲染分开：先前选中项变化走的是全量重建，而重建后从未把当前行
 * 滚回可视区，于是按 j 往下走时高亮会跑到列表可视范围之外（用户看到的就是
 * 「选中项越过第二栏底线」）。
 */
function focusRow(id, { follow = false } = {}) {
  const list = el('entries');
  for (const li of list.children) {
    const isActive = id != null && li.dataset.id === String(id);
    li.classList.toggle('active', isActive);
    // block:'nearest' —— 已在视野内就不动，避免每次按键都把列表拽一下
    if (isActive && follow) li.scrollIntoView({ block: 'nearest' });
  }
}

/** 单独把某行的「已读」样式更新掉（同样不重建列表） */
function markRowRead(id) {
  for (const li of el('entries').children) {
    if (li.dataset.id === String(id)) li.classList.add('read');
  }
}

function renderReaderEmpty() {
  el('reader').innerHTML = `<div class="reader-empty">
      <p>${t('reader.empty')}</p>
      <p class="dim">${t('reader.shortcuts')}</p>
    </div>`;
}

function renderReader(entry) {
  const reader = el('reader');
  const body = entry.content_html
    ? sanitize(entry.content_html, entry.url)
    : (entry.content_text || '')
        .split(/\n{1,}/)
        .map((p) => `<p>${escapeHtml(p)}</p>`)
        .join('');

  reader.innerHTML = `
    <div class="reader-head">
      <h1>${escapeHtml(entry.title)}</h1>
      <div class="meta">
        <span>${escapeHtml(entry.feed_title)}</span>
        <span>${fmtTime(entry.published_at)}</span>
        ${entry.author ? `<span>${escapeHtml(entry.author)}</span>` : ''}
      </div>
    </div>
    <div class="reader-actions">
      <button id="act-read">${entry.read ? t('reader.markUnread') : t('reader.markRead')}</button>
      <button id="act-star">${entry.starred ? t('reader.removeStar') : t('reader.addStar')}</button>
      ${entry.url ? `<button id="act-open">${t('reader.openInBrowser')}</button><button id="act-copy">${t('reader.copyLink')}</button>` : ''}
      <button id="act-summarize" title="${t('reader.summarizeTitle')}">${t('reader.summarize')}</button>
      <button id="act-translate" title="${t('reader.translateTitle')}">${t('reader.translate')}</button>
    </div>
    <div id="ai-panel" class="ai-panel hidden">
      <div class="ai-panel-head">
        <b id="ai-panel-title"></b>
        <span id="ai-panel-meta" class="dim"></span>
        <span class="grow"></span>
        <button id="ai-regenerate">${t('ai.panel.regenerate')}</button>
        <button id="ai-close">${t('ai.panel.close')}</button>
      </div>
      <div id="ai-panel-body" class="ai-panel-body"></div>
    </div>
    <div class="article">${body}</div>`;

  el('act-read').onclick = () => toggleRead();
  el('act-star').onclick = () => toggleStar();
  el('act-summarize').onclick = () => runAi('summarize');
  el('act-translate').onclick = () => runAi('translate');
  el('ai-regenerate').onclick = () => runAi(currentAiTask, { refresh: true });
  el('ai-close').onclick = () => el('ai-panel').classList.add('hidden');
  if (entry.url) {
    el('act-open').onclick = () => invoke('open_external', { url: entry.url }).catch((e) => setStatus(e.message, true));
    el('act-copy').onclick = () =>
      invoke('clip_write', { text: entry.url })
        .then(() => setStatus(t('reader.linkCopied')))
        .catch((e) => setStatus(t('reader.copyFailed', { error: e.message }), true));
  }

  // 正文里的链接交给系统浏览器，避免在应用内导航走丢
  reader.querySelectorAll('a[href]').forEach((a) => {
    a.onclick = (ev) => {
      ev.preventDefault();
      invoke('open_external', { url: a.getAttribute('href') }).catch((e) => setStatus(e.message, true));
    };
  });
  reader.scrollTop = 0;
}

// ---------------------------------------------------------------- 数据流

async function loadAll() {
  const [db, feeds, settings, ai, mcp] = await Promise.all([
    invoke('db_info'),
    invoke('list_feeds'),
    invoke('get_ui_settings'),
    invoke('get_ai_settings'),
    invoke('get_mcp_settings'),
  ]);
  state.db = db;
  state.feeds = feeds;
  state.settings = settings;
  state.ai = ai;
  state.mcp = mcp;
  // 语言设置来自数据库；先应用再渲染，避免先闪一下默认语言
  setLocale(settings.locale || 'auto');
  applyStaticI18n();
  renderSidebar();
  await loadEntries();
  log(
    `loaded feeds=${db.feeds} entries=${db.entries} unread=${db.unread} starred=${db.starred} markReadOnNavigate=${settings.mark_read_on_navigate} ai=${ai.provider}${ai.model ? '/' + ai.model : '（未配模型）'} hasKey=${ai.has_key} mcp=${mcp.running ? mcp.url : 'off'}`
  );
}

async function loadEntries() {
  const kind = state.view.kind;
  let rows;
  if (kind === 'search') {
    rows = state.query ? await invoke('search', { query: state.query, limit: 200 }) : [];
  } else {
    rows = await invoke('list_entries', {
      feedId: kind === 'feed' ? state.feedId : null,
      unreadOnly: kind === 'unread',
      starredOnly: kind === 'starred',
      limit: 200,
    });
  }
  state.entries = rows;
  if (!rows.some((e) => e.id === state.selectedId)) {
    state.selectedId = rows.length ? rows[0].id : null;
  }
  renderList();
  if (state.selectedId) {
    // 视图切换只加载，不标记已读
    const entry = await invoke('get_entry', { id: state.selectedId });
    if (entry) renderReader(entry);
    focusRow(state.selectedId, { follow: false });
  } else {
    renderReaderEmpty();
  }
  log(`view=${kind}${state.feedId ? '#' + state.feedId : ''} count=${rows.length}`);
}

async function setView(view) {
  state.view = view;
  state.feedId = view.feedId ?? null;
  if (view.kind !== 'search') {
    el('search').value = '';
    state.query = '';
  }
  renderSidebar();
  await loadEntries();
}

/** 打开某篇文章；markRead=true 表示这是用户主动打开的动作 */
async function openEntry(id, { markRead, follow = true } = {}) {
  const entry = await invoke('get_entry', { id });
  if (!entry) return;
  const changed = state.selectedId !== id;
  state.selectedId = id;
  if (changed) focusRow(id, { follow });
  renderReader(entry);

  if (markRead && !entry.read) {
    await invoke('set_read', { ids: [id], read: true });
    const row = state.entries.find((e) => e.id === id);
    if (row) row.read = true;

    if (state.view.kind === 'unread') {
      // 未读视图里读过的文章会离开列表 → 这时才需要重建，并保持阅读位置
      const idx = state.entries.findIndex((e) => e.id === id);
      state.entries = state.entries.filter((e) => e.id !== id);
      const next = state.entries[Math.min(idx, state.entries.length - 1)];
      state.selectedId = next ? next.id : null;
      renderList();
      if (next) {
        const fresh = await invoke('get_entry', { id: next.id });
        if (fresh) renderReader(fresh);
      } else {
        renderReaderEmpty();
      }
    } else {
      markRowRead(id);
    }
    await refreshCounts();
  }
  log(`open id=${id} markRead=${markRead} read=${entry.read}`);
}

async function refreshCounts() {
  const [db, feeds] = await Promise.all([invoke('db_info'), invoke('list_feeds')]);
  state.db = db;
  state.feeds = feeds;
  renderSidebar();
}

function move(delta) {
  if (!state.entries.length) return;
  const idx = state.entries.findIndex((e) => e.id === state.selectedId);
  const next = Math.max(0, Math.min(state.entries.length - 1, (idx < 0 ? 0 : idx) + delta));
  // j/k 是否顺便标已读完全取决于设置（默认开）
  openEntry(state.entries[next].id, { markRead: state.settings.mark_read_on_navigate });
}

function jump(toEnd) {
  if (!state.entries.length) return;
  const target = toEnd ? state.entries[state.entries.length - 1] : state.entries[0];
  openEntry(target.id, { markRead: state.settings.mark_read_on_navigate });
}

async function toggleRead() {
  const row = state.entries.find((e) => e.id === state.selectedId);
  if (!row) return;
  const read = !row.read;
  await invoke('set_read', { ids: [row.id], read });
  row.read = read;
  // 未读视图下标记已读后，该条应从列表消失
  if (state.view.kind === 'unread' && read) {
    // 未读视图下标记已读 → 该条离开列表，保持阅读位置（选原位置的下一行，不跳回顶部）
    const idx = state.entries.findIndex((e) => e.id === row.id);
    state.entries = state.entries.filter((e) => e.id !== row.id);
    const next = state.entries[Math.min(idx, state.entries.length - 1)];
    state.selectedId = next ? next.id : null;
    renderList();
    if (next) await openEntry(next.id, { markRead: false });
    else renderReaderEmpty();
  } else {
    const fresh = await invoke('get_entry', { id: row.id });
    if (fresh) renderReader(fresh);
    // 只改那一行的已读样式，不重建列表
    for (const li of el('entries').children) {
      if (li.dataset.id === String(row.id)) li.classList.toggle('read', read);
    }
  }
  await refreshCounts();
}

async function toggleStar() {
  const row = state.entries.find((e) => e.id === state.selectedId);
  if (!row) return;
  const starred = !row.starred;
  await invoke('set_starred', { ids: [row.id], starred });
  row.starred = starred;
  const fresh = await invoke('get_entry', { id: row.id });
  if (fresh) renderReader(fresh);
  renderList();
  await refreshCounts();
}

async function doRefresh() {
  const btn = el('btn-refresh');
  btn.disabled = true;
  setStatus(t('status.refreshing'));
  try {
    const r = await invoke('refresh_all', { concurrency: 6 });
    const summary = t('status.refreshDone', {
      fetched: r.fetched,
      notModified: r.not_modified,
      inserted: r.inserted,
      failures: r.failures.length,
    });
    setStatus(summary);
    log(`refresh_all ${summary}`);
    for (const f of r.failures) log(`refresh failure feed=${f.feed_id} ${f.url} :: ${f.error}`);
    await loadAll();
  } catch (e) {
    setStatus(t('status.refreshFailed', { error: e.message }), true);
    log(`refresh_all failed: ${e.message}`);
  } finally {
    btn.disabled = false;
  }
}

async function refreshOne(feedId) {
  setStatus(t('status.refreshingOne'));
  try {
    const r = await invoke('refresh_feed', { feedId, concurrency: 1 });
    setStatus(
      t('status.refreshOneDone', {
        inserted: r.inserted,
        notModified: r.not_modified,
        failures: r.failures.length,
      })
    );
    await loadAll();
  } catch (e) {
    setStatus(t('status.refreshFailed', { error: e.message }), true);
  }
}

async function doAddFeed() {
  const input = el('add-url');
  const btn = el('add-ok');
  const url = input.value.trim();
  if (!url) return;

  // 第一步：发现。站点首页在这里换成真正的 feed 地址；输入本身就是 feed 则原样返回。
  // 失败时输入原样留着、按钮恢复可用，改一下再点就是重试。
  btn.disabled = true;
  let found;
  try {
    setStatus(t('status.discovering'));
    found = await invoke('discover_feed', { url });
    log(
      `discover_feed ${url} -> ${found.feed_url} via=${found.via} alternatives=${found.alternatives.length}`
    );
  } catch (e) {
    setStatus(t('status.discoverFailed', { error: e.message }), true);
    log(`discover_feed failed: ${e.message}`);
    btn.disabled = false;
    return;
  }

  // 第二步：订阅 + 首次抓取，沿用原有路径（订阅地址是发现出来的那个）
  try {
    const id = await invoke('add_feed', { url: found.feed_url });
    setStatus(t('status.adding'));
    log(`add_feed id=${id} url=${found.feed_url}`);
    const r = await invoke('refresh_feed', { feedId: id, concurrency: 1 });
    setStatus(t('status.added', { inserted: r.inserted }));
    input.value = '';
    await loadAll();
  } catch (e) {
    setStatus(t('status.addFailed', { error: e.message }), true);
    log(`add_feed failed: ${e.message}`);
  } finally {
    btn.disabled = false;
  }
}

// ---------------------------------------------------------------- 启动与键盘

/**
 * 启动自检：验证「feed 里的脚本不会活下来」与「相对地址会被解析」两条。
 * 结果通过 ui_log 上报，因此不需要人工看界面也能确认——这是 spec 里的硬验收点。
 */
function selfTestSanitizer() {
  const base = 'https://example.com/posts/1';
  const dirty = `
    <div><p>正常段落</p><script>window.__pwned = 1<\/script>
    <style>p{color:red}</style>
    <img src="img/a.png" onerror="window.__pwned=2">
    <a href="javascript:window.__pwned=3">js链接</a>
    <a href="/about">相对链接</a>
    <iframe src="https://evil.example"></iframe></div>`;
  const clean = sanitize(dirty, base);

  const failures = [];
  if (/<script/i.test(clean)) failures.push('script 标签残留');
  if (/<style/i.test(clean)) failures.push('style 标签残留');
  if (/onerror/i.test(clean)) failures.push('事件处理器残留');
  if (/javascript:/i.test(clean)) failures.push('javascript: 链接残留');
  if (/<iframe/i.test(clean)) failures.push('iframe 残留');
  if (!clean.includes('正常段落')) failures.push('正常内容被误删');
  if (!clean.includes('https://example.com/posts/img/a.png')) failures.push('相对图片未解析为绝对地址');
  if (!clean.includes('https://example.com/about')) failures.push('相对链接未解析为绝对地址');
  if (window.__pwned) failures.push('脚本被实际执行了');

  log(failures.length ? `sanitizer selftest FAILED: ${failures.join('; ')}` : 'sanitizer selftest ok');
  return failures.length === 0;
}

async function markAll(read) {
  const feedId = state.view.kind === 'feed' ? state.feedId : null;
  const cmd = read ? 'mark_all_read' : 'mark_all_unread';
  try {
    const n = await invoke(cmd, { feedId });
    setStatus(t(read ? 'status.markedRead' : 'status.markedUnread', { n }));
    log(`${cmd} scope=${feedId ?? 'all'} changed=${n}`);
    el('settings-overlay').classList.add('hidden');
    await loadAll();
  } catch (e) {
    setStatus(t('status.markFailed', { error: e.message }), true);
  }
}

let currentAiTask = 'summarize';

/// 弹「发送前确认要发什么」：resolve(true)=发送，resolve(false)=取消
function confirmAiSend(p) {
  return new Promise((resolve) => {
    const overlay = el('ai-confirm-overlay');
    el('ai-confirm-summary').textContent = t('ai.confirm.summary', {
      model: p.provider_model,
      chars: String(p.body_chars),
    });
    el('ai-confirm-url').textContent = p.url;
    el('ai-confirm-headers').textContent = p.headers.map(([k, v]) => `${k}: ${v}`).join('\n');
    el('ai-confirm-body').textContent = p.body;
    el('ai-confirm-meta').textContent = p.body_clipped
      ? t('ai.confirm.clipped', {
          // 用码点计数：跟后端 chars().count() 同口径，否则含 emoji 时会出现 shown > total
          shown: String([...p.body].length),
          total: String(p.body_chars),
        })
      : '';
    el('ai-confirm-note').textContent = p.truncated ? t('ai.confirm.truncated') : '';
    el('ai-confirm-dont-ask').checked = false;
    overlay.classList.remove('hidden');
    el('ai-confirm-send').focus();

    const finish = (ok) => {
      overlay.classList.add('hidden');
      el('ai-confirm-send').onclick = null;
      el('ai-confirm-cancel').onclick = null;
      document.removeEventListener('keydown', onKey, true);
      resolve(ok);
    };

    el('ai-confirm-send').onclick = () => {
      if (el('ai-confirm-dont-ask').checked) {
        invoke('set_ai_confirm_before_send', { enabled: false })
          .then((view) => {
            state.ai = view;
            log('ai confirm_before_send=false（不再询问）');
          })
          .catch((err) => log(`set confirm_before_send failed: ${err.message || err}`));
      }
      finish(true);
    };
    el('ai-confirm-cancel').onclick = () => finish(false);

    function onKey(e) {
      if (e.key === 'Escape') {
        e.preventDefault();
        // 捕获阶段就阻断：否则同一事件冒泡到全局 Escape 处理器时，
        // 弹窗已经隐藏、它的 guard 失效，会顺手清空搜索框/切换视图
        e.stopPropagation();
        finish(false);
      } else if (e.key === 'Tab') {
        // 简易焦点圈：不让 Tab 把焦点移到弹窗背后（那里 Enter 会改选中项）
        const focusable = overlay.querySelectorAll('button, input, [tabindex]:not([tabindex="-1"])');
        if (!focusable.length) return;
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        if (!overlay.contains(document.activeElement)) {
          e.preventDefault();
          first.focus();
        } else if (e.shiftKey && document.activeElement === first) {
          e.preventDefault();
          last.focus();
        } else if (!e.shiftKey && document.activeElement === last) {
          e.preventDefault();
          first.focus();
        }
      }
      // Enter 交给按钮原生行为：焦点在哪个按钮就触发哪个。
      // 自己抢 Enter 会出现「Tab 到取消键按回车却发送」这种反向行为。
    }

    // 用 document 捕获阶段监听：焦点可能落到背板（body），挂 overlay 会漏掉 Esc
    document.addEventListener('keydown', onKey, true);
  });
}

/** 跑一次 AI 任务并把结果放上面板。结果是否来自缓存会如实标出来。 */
async function runAi(kind, { refresh = false } = {}) {
  if (!state.selectedId) return;
  // 快照：确认框开着的时候选中项可能变（Tab 逃逸/鼠标），但用户确认的是这一篇
  const entryId = state.selectedId;
  currentAiTask = kind === 'translate' ? 'translate' : 'summarize';
  const panel = el('ai-panel');
  panel.classList.remove('hidden');
  el('ai-panel-title').textContent =
    currentAiTask === 'summarize' ? t('ai.panel.summary') : t('ai.panel.translate');
  el('ai-panel-meta').textContent = t('ai.panel.requesting');
  el('ai-panel-body').textContent = '';

  try {
    // 「发送前确认要发什么」：先问后端即将发出的请求长什么样（凭据已打码）
    if (state.ai && state.ai.confirm_before_send) {
      let preview = null;
      try {
        // refresh 要传给预览：重新生成会忽略缓存，预览必须用同一策略
        preview = await invoke('ai_preview', { entryId, task: currentAiTask, refresh });
      } catch (err) {
        // 预览失败不拦路（如尚未配 key），交给真实调用报错：那个理由更具体
        log(`ai preview failed: ${err.message || err}`);
      }
      if (preview) {
        log(
          `ai preview send=${preview.will_send} cache=${preview.from_cache} body=${preview.body_chars} truncated=${preview.truncated}`
        );
        if (!preview.will_send) {
          // 命中缓存 → 根本不会外发，不需要问（仅 refresh=false 时会出现）
          setStatus(t('status.aiCachedNoSend'));
        } else if (!(await confirmAiSend(preview))) {
          panel.classList.add('hidden');
          setStatus(t('status.aiCancelled'));
          log('ai send cancelled by user');
          return;
        }
      }
    }
    const cmd = currentAiTask === 'summarize' ? 'ai_summarize' : 'ai_translate';
    const r = await invoke(cmd, { entryId, refresh });
    el('ai-panel-meta').textContent = `${r.provider_model}｜${
      r.from_cache ? t('ai.panel.fromCache') : t('ai.panel.fresh')
    }${r.truncated ? '｜' + t('ai.panel.truncated') : ''}`;
    el('ai-panel-body').textContent = r.output;
    log(
      `ai ${currentAiTask} entry=${entryId} from_cache=${r.from_cache} chars=${r.output.length} model=${r.provider_model}`
    );
  } catch (e) {
    el('ai-panel-meta').textContent = t('ai.panel.failed');
    el('ai-panel-body').textContent = e.message;
    log(`ai ${currentAiTask} failed: ${e.message}`);
  }
}

function fillAiForm() {
  const ai = state.ai;
  if (!ai) return;
  el('ai-provider').value = ai.provider;
  el('ai-model').value = ai.model;
  el('ai-base-url').value = ai.base_url;
  el('ai-target').value = ai.translate_target;
  el('ai-key').value = '';
  el('ai-key-hint').textContent = ai.has_key
    ? t('settings.ai.keySet', { source: ai.key_note || '' })
    : t('settings.ai.keyUnset');
  el('ai-status').textContent = t('settings.ai.status', {
    base: ai.default_base_url,
    key: ai.has_key ? t('settings.ai.statusSet') : t('settings.ai.statusUnset'),
  });
  el('set-ai-confirm').checked = !!ai.confirm_before_send;
}

function fillMcpForm() {
  const mcp = state.mcp;
  if (!mcp) return;
  el('set-mcp-enabled').checked = mcp.enabled;
  el('mcp-port').value = mcp.port;
  el('mcp-snippet').value = mcp.snippet || '';
  // token 只显示首尾：设置页不需要完整明文，需要时用「复制客户端配置」
  const masked = mcp.token ? `${mcp.token.slice(0, 6)}…${mcp.token.slice(-4)}` : '(无)';
  el('mcp-status').textContent = mcp.running
    ? t('settings.mcp.statusRunning', {
        url: mcp.url,
        loopback: mcp.loopback_only ? t('common.yes') : t('common.no'),
        token: masked,
      })
    : t('settings.mcp.statusStopped', { token: masked });
}

/** 切到某个设置分类（左栏可选，右栏只显示对应面板） */
function showPane(name) {
  for (const li of el('settings-nav-list').children) {
    li.classList.toggle('active', li.dataset.pane === name);
  }
  for (const pane of document.querySelectorAll('.settings-body .pane')) {
    pane.classList.toggle('hidden', pane.dataset.pane !== name);
  }
  document.querySelector('.settings-body').scrollTop = 0;
}

/// 记住上次看的是哪个分类（同一次运行内）
let currentPane = 'general';

function openSettings() {
  el('set-mark-read').checked = state.settings.mark_read_on_navigate;
  el('set-language').value = state.settings.locale || 'auto';
  const dbPath = state.db ? state.db.dbPath : '';
  el('settings-db-path').textContent = dbPath;
  el('settings-db-path').title = dbPath;
  el('general-db-path').textContent = dbPath;
  fillAiForm();
  fillMcpForm();
  showPane(currentPane);
  el('settings-overlay').classList.remove('hidden');
}

async function boot() {
  applyStaticI18n();
  const i18n = i18nSelfTest();
  log(
    i18n.ok
      ? `i18n selftest ok (keys=${i18n.keys})`
      : `i18n selftest FAILED: ${i18n.problems.join('; ')}`
  );
  selfTestSanitizer();
  try {
    await loadAll();
  } catch (e) {
    setStatus(t('status.bootFailed', { error: e.message }), true);
    log(`boot failed: ${e.message}`);
    return;
  }

  el('btn-refresh').onclick = doRefresh;
  el('btn-add').onclick = () => {
    const row = el('add-row');
    row.classList.toggle('hidden');
    if (!row.classList.contains('hidden')) el('add-url').focus();
  };
  el('btn-settings').onclick = openSettings;
  el('ai-save').onclick = async () => {
    const key = el('ai-key').value;
    try {
      state.ai = await invoke('save_ai_settings', {
        provider: el('ai-provider').value,
        model: el('ai-model').value,
        baseUrl: el('ai-base-url').value,
        translateTarget: el('ai-target').value,
        // 留空表示「不修改 key」，要清除得点专门的按钮
        apiKey: key.trim() === '' ? null : key,
      });
      fillAiForm();
      el('ai-status').textContent = state.ai.has_key
        ? t('settings.ai.savedWithKey')
        : t('settings.ai.savedNoKey');
      log(`ai saved provider=${state.ai.provider} model=${state.ai.model} has_key=${state.ai.has_key}`);
    } catch (e) {
      el('ai-status').textContent = t('settings.ai.saveFailed', { error: e.message });
      log(`ai save failed: ${e.message}`);
    }
  };

  el('ai-clear-key').onclick = async () => {
    try {
      state.ai = await invoke('save_ai_settings', {
        provider: el('ai-provider').value,
        model: el('ai-model').value,
        baseUrl: el('ai-base-url').value,
        translateTarget: el('ai-target').value,
        apiKey: '',
      });
      fillAiForm();
      el('ai-status').textContent = t('settings.ai.keyCleared');
      log('ai key cleared');
    } catch (e) {
      el('ai-status').textContent = t('settings.ai.clearFailed', { error: e.message });
    }
  };

  el('ai-test').onclick = async () => {
    el('ai-status').textContent = t('settings.ai.testing');
    try {
      const reply = await invoke('test_ai_connection');
      el('ai-status').textContent = t('settings.ai.testOk', { reply });
      log(`ai test ok reply=${reply.slice(0, 40)}`);
    } catch (e) {
      el('ai-status').textContent = t('settings.ai.testFailed', { error: e.message });
      log(`ai test failed: ${e.message}`);
    }
  };

  el('settings-close').onclick = () => el('settings-overlay').classList.add('hidden');

  // 左栏分类切换（事件委派：以后加分类不用改这里）
  el('settings-nav-list').addEventListener('click', (e) => {
    const li = e.target.closest('li[data-pane]');
    if (!li) return;
    currentPane = li.dataset.pane;
    showPane(currentPane);
    log(`settings pane=${currentPane}`);
  });

  el('set-language').addEventListener('change', async (e) => {    try {
      state.settings = await invoke('set_ui_locale', { locale: e.target.value });
      setLocale(state.settings.locale || 'auto');
      applyStaticI18n();
      // 动态文案需要重渲染
      renderSidebar();
      renderList();
      if (state.selectedId) {
        const entry = await invoke('get_entry', { id: state.selectedId });
        if (entry) renderReader(entry);
      } else {
        renderReaderEmpty();
      }
      fillAiForm();
      log(`locale=${state.settings.locale} → ${currentLocale()}`);
    } catch (err) {
      setStatus(t('status.settingFailed', { error: err.message }), true);
    }
  });
  el('settings-overlay').addEventListener('click', (e) => {
    // 点击遮罩区域关闭（点对话框内部不关）
    if (e.target === el('settings-overlay')) el('settings-overlay').classList.add('hidden');
  });
  el('set-mark-read').addEventListener('change', async (e) => {
    try {
      state.settings = await invoke('set_mark_read_on_navigate', { enabled: e.target.checked });
      setStatus(t(e.target.checked ? 'status.markReadOn' : 'status.markReadOff'));
      log(`setting mark_read_on_navigate=${e.target.checked}`);
    } catch (err) {
      setStatus(t('status.settingFailed', { error: err.message }), true);
    }
  });
  el('act-mark-all-read').onclick = () => markAll(true);
  el('act-mark-all-unread').onclick = () => markAll(false);

  el('act-export-opml').onclick = async () => {    try {
      const path = await invoke('export_opml');
      setStatus(path ? t('status.exported', { path }) : t('status.exportCancelled'));
      log(`export_opml ${path ?? 'cancelled'}`);
    } catch (err) {
      setStatus(t('status.exportFailed', { error: err.message }), true);
      log(`export_opml failed: ${err.message}`);
    }
  };

  el('act-import-opml').onclick = async () => {
    try {
      const r = await invoke('import_opml');
      if (!r) {
        setStatus(t('status.importCancelled'));
        return;
      }
      setStatus(
        t('status.imported', {
          added: r.feeds_added,
          skipped: r.feeds_skipped,
          folders: r.folders_created,
          ignored: r.outlines_ignored,
        })
      );
      log(
        `import_opml added=${r.feeds_added} skipped=${r.feeds_skipped} folders=${r.folders_created} ignored=${r.outlines_ignored}`
      );
      el('settings-overlay').classList.add('hidden');
      await loadAll();
    } catch (err) {
      setStatus(t('status.importFailed', { error: err.message }), true);
      log(`import_opml failed: ${err.message}`);
    }
  };
  el('add-ok').onclick = doAddFeed;
  el('add-url').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') doAddFeed();
    if (e.key === 'Escape') el('add-row').classList.add('hidden');
  });

  el('set-mcp-enabled').addEventListener('change', async (e) => {
    try {
      state.mcp = await invoke('set_mcp_enabled', { enabled: e.target.checked });
      fillMcpForm();
      setStatus(e.target.checked && state.mcp.running ? `${state.mcp.url}` : '');
      log(
        `mcp enabled=${e.target.checked} running=${state.mcp.running} url=${state.mcp.url ?? '-'} port=${state.mcp.port}`
      );
    } catch (err) {
      e.target.checked = false;
      el('mcp-status').textContent = t('settings.mcp.failed', { error: err.message });
      log(`mcp enable failed: ${err.message}`);
    }
  });

  // 「发送前确认要发什么」开关：不需要重传整张 AI 表单，单独存
  el('set-ai-confirm').addEventListener('change', async (e) => {
    try {
      state.ai = await invoke('set_ai_confirm_before_send', { enabled: e.target.checked });
      fillAiForm();
      log(`ai confirm_before_send=${state.ai.confirm_before_send}`);
    } catch (err) {
      e.target.checked = !e.target.checked;
      el('ai-status').textContent = err.message;
      log(`ai confirm_before_send failed: ${err.message}`);
    }
  });

  el('mcp-port').addEventListener('change', async (e) => {
    const port = Number(e.target.value);
    try {
      state.mcp = await invoke('set_mcp_port', { port });
      fillMcpForm();
      log(`mcp port=${state.mcp.port} running=${state.mcp.running}`);
    } catch (err) {
      el('mcp-status').textContent = t('settings.mcp.failed', { error: err.message });
      log(`mcp port failed: ${err.message}`);
    }
  });

  el('mcp-copy-snippet').onclick = async () => {
    try {
      await invoke('clip_write', { text: el('mcp-snippet').value });
      setStatus(t('settings.mcp.copied'));
      log('mcp snippet copied');
    } catch (err) {
      setStatus(t('settings.mcp.copyFailed', { error: err.message }), true);
    }
  };

  el('mcp-rotate').onclick = async () => {
    try {
      state.mcp = await invoke('rotate_mcp_token');
      fillMcpForm();
      setStatus(t('settings.mcp.rotated'));
      log(`mcp token rotated running=${state.mcp.running}`);
    } catch (err) {
      el('mcp-status').textContent = t('settings.mcp.failed', { error: err.message });
      log(`mcp rotate failed: ${err.message}`);
    }
  };

  let searchTimer = null;
  el('search').addEventListener('input', (e) => {
    const value = e.target.value.trim();
    clearTimeout(searchTimer);
    searchTimer = setTimeout(async () => {
      state.query = value;
      if (value) {
        state.view = { kind: 'search' };
        renderSidebar();
        await loadEntries();
      } else {
        await setView({ kind: 'unread' });
      }
    }, 250);
  });

  document.addEventListener('keydown', (e) => {
    const inField = ['INPUT', 'TEXTAREA'].includes(document.activeElement?.tagName);
    // 确认框自己处理 Esc/Enter，其它全局快捷键先让位
    if (!el('ai-confirm-overlay').classList.contains('hidden')) return;
    if (e.key === '/' && !inField) {
      e.preventDefault();
      el('search').focus();
      return;
    }
    if (e.key === 'Escape') {
      el('settings-overlay').classList.add('hidden');
      el('search').value = '';
      el('search').blur();
      el('add-row').classList.add('hidden');
      if (state.view.kind === 'search') setView({ kind: 'unread' });
      return;
    }
    if (inField || e.ctrlKey || e.metaKey || e.altKey) return;
    // 设置面板开着时，导航类快捷键同样让位（否则 j/k 会在面板背后换文章）
    if (!el('settings-overlay').classList.contains('hidden')) return;

    switch (e.key) {
      case 'j': case 'ArrowDown': e.preventDefault(); move(1); break;
      case 'k': case 'ArrowUp': e.preventDefault(); move(-1); break;
      case 'Enter': e.preventDefault(); if (state.selectedId) openEntry(state.selectedId, { markRead: true }); break;
      case 'u': e.preventDefault(); toggleRead().catch((err) => setStatus(err.message, true)); break;
      case 's': e.preventDefault(); toggleStar().catch((err) => setStatus(err.message, true)); break;
      case 'r': e.preventDefault(); doRefresh(); break;
      case 'g': e.preventDefault(); jump(false); break;
      case 'G': e.preventDefault(); jump(true); break;
      default: break;
    }
  });
}

window.addEventListener('DOMContentLoaded', boot);

})();
