// RustRss 界面逻辑。
//
// 两条刻意的设计选择：
// 1. **只有用户主动打开文章才标记已读**（点击 / j / k / Enter）。切换视图或刷新只是
//    加载列表，不会顺手把没看过的文章标成已读。
// 2. 正文一律经白名单清洗后再插入 DOM。feed 是不可信输入，绝不能让它执行脚本。

const el = (id) => document.getElementById(id);

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
};

const VIEWS = [
  { kind: 'unread', label: '全部未读', icon: '●' },
  { kind: 'starred', label: '星标', icon: '★' },
  { kind: 'all', label: '全部', icon: '≡' },
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
  return sameDay
    ? d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString('zh-CN', { year: 'numeric', month: '2-digit', day: '2-digit' });
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
    li.innerHTML = `<span class="icon">${v.icon}</span><span>${v.label}</span><span class="count">${counts[v.kind]}</span>`;
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
      ? `上次抓取：${f.last_status}｜${f.last_error || ''}\n双击可重试`
      : `${f.url}\n双击刷新此源`;
    li.innerHTML = `<span class="name">${escapeHtml(f.title)}</span>${failed ? '<span class="dot">●</span>' : ''}<span class="count">${f.unread}</span>`;
    li.onclick = () => setView({ kind: 'feed', feedId: f.id });
    li.ondblclick = () => refreshOne(f.id);
    feeds.appendChild(li);
  }
  el('feeds-meta').textContent = `${state.feeds.length} 个`;
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
  if (state.view.kind === 'search') return `搜索：${state.query}`;
  if (state.view.kind === 'feed') {
    const feed = state.feeds.find((f) => f.id === state.feedId);
    return feed ? feed.title : '订阅源';
  }
  return VIEWS.find((v) => v.kind === state.view.kind)?.label ?? '文章';
}

function renderList() {
  el('list-title').textContent = viewTitle();
  el('list-count').textContent = state.entries.length ? `${state.entries.length} 篇` : '';

  const list = el('entries');
  list.innerHTML = '';
  if (!state.entries.length) {
    const li = document.createElement('li');
    li.className = 'dim';
    li.style.cursor = 'default';
    li.textContent = state.view.kind === 'unread' ? '没有未读文章' : '这里还没有文章';
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
      <p>从中间列表选一篇文章。</p>
      <p class="dim">快捷键：<b>j</b>/<b>k</b> 上下 · <b>Enter</b> 打开 · <b>u</b> 未读切换 ·
      <b>s</b> 星标 · <b>r</b> 刷新 · <b>/</b> 搜索 · <b>Esc</b> 清除搜索</p>
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
      <button id="act-read">${entry.read ? '标为未读' : '标为已读'}</button>
      <button id="act-star">${entry.starred ? '取消星标' : '加星标'}</button>
      ${entry.url ? '<button id="act-open">浏览器打开</button><button id="act-copy">复制链接</button>' : ''}
    </div>
    <div class="article">${body}</div>`;

  el('act-read').onclick = () => toggleRead();
  el('act-star').onclick = () => toggleStar();
  if (entry.url) {
    el('act-open').onclick = () => invoke('open_external', { url: entry.url }).catch((e) => setStatus(e.message, true));
    el('act-copy').onclick = () =>
      invoke('clip_write', { text: entry.url })
        .then(() => setStatus('链接已复制'))
        .catch((e) => setStatus('复制失败：' + e.message, true));
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
  const [db, feeds, settings] = await Promise.all([
    invoke('db_info'),
    invoke('list_feeds'),
    invoke('get_ui_settings'),
  ]);
  state.db = db;
  state.feeds = feeds;
  state.settings = settings;
  renderSidebar();
  await loadEntries();
  log(
    `loaded feeds=${db.feeds} entries=${db.entries} unread=${db.unread} starred=${db.starred} markReadOnNavigate=${settings.mark_read_on_navigate}`
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
  setStatus('正在刷新…');
  try {
    const r = await invoke('refresh_all', { concurrency: 6 });
    const summary = `成功 ${r.fetched}｜未修改 ${r.not_modified}｜新增 ${r.inserted}｜失败 ${r.failures.length}`;
    setStatus('刷新完成：' + summary);
    log(`refresh_all ${summary}`);
    for (const f of r.failures) log(`refresh failure feed=${f.feed_id} ${f.url} :: ${f.error}`);
    await loadAll();
  } catch (e) {
    setStatus('刷新失败：' + e.message, true);
    log(`refresh_all failed: ${e.message}`);
  } finally {
    btn.disabled = false;
  }
}

async function refreshOne(feedId) {
  setStatus('正在刷新该源…');
  try {
    const r = await invoke('refresh_feed', { feedId, concurrency: 1 });
    setStatus(`该源刷新完成：新增 ${r.inserted}｜未修改 ${r.not_modified}｜失败 ${r.failures.length}`);
    await loadAll();
  } catch (e) {
    setStatus('刷新失败：' + e.message, true);
  }
}

async function doAddFeed() {
  const input = el('add-url');
  const url = input.value.trim();
  if (!url) return;
  try {
    const id = await invoke('add_feed', { url });
    setStatus('已添加，正在抓取…');
    log(`add_feed id=${id} url=${url}`);
    const r = await invoke('refresh_feed', { feedId: id, concurrency: 1 });
    setStatus(`已添加：新增 ${r.inserted} 篇`);
    input.value = '';
    await loadAll();
  } catch (e) {
    setStatus('添加失败：' + e.message, true);
    log(`add_feed failed: ${e.message}`);
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
    setStatus(`${read ? '已标为已读' : '已标为未读'} ${n} 篇`);
    log(`${cmd} scope=${feedId ?? 'all'} changed=${n}`);
    el('settings-overlay').classList.add('hidden');
    await loadAll();
  } catch (e) {
    setStatus('操作失败：' + e.message, true);
  }
}

function openSettings() {
  el('set-mark-read').checked = state.settings.mark_read_on_navigate;
  el('settings-overlay').classList.remove('hidden');
}

async function boot() {
  selfTestSanitizer();
  try {
    await loadAll();
  } catch (e) {
    setStatus('初始化失败：' + e.message, true);
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
  el('settings-close').onclick = () => el('settings-overlay').classList.add('hidden');
  el('settings-overlay').addEventListener('click', (e) => {
    // 点击遮罩区域关闭（点对话框内部不关）
    if (e.target === el('settings-overlay')) el('settings-overlay').classList.add('hidden');
  });
  el('set-mark-read').addEventListener('change', async (e) => {
    try {
      state.settings = await invoke('set_mark_read_on_navigate', { enabled: e.target.checked });
      setStatus(`已${e.target.checked ? '开启' : '关闭'}「j/k 浏览时标记已读」`);
      log(`setting mark_read_on_navigate=${e.target.checked}`);
    } catch (err) {
      setStatus('保存设置失败：' + err.message, true);
    }
  });
  el('act-mark-all-read').onclick = () => markAll(true);
  el('act-mark-all-unread').onclick = () => markAll(false);
  el('add-ok').onclick = doAddFeed;
  el('add-url').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') doAddFeed();
    if (e.key === 'Escape') el('add-row').classList.add('hidden');
  });

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
