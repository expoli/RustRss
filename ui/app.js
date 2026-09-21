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

/// 对正文里的代码块做语法高亮：有 language-* class（sanitize 白名单放行的）
/// 按指定语言，否则 hljs auto-detect；任何失败都让代码块保持原样纯文本。
/// hljs 未加载（如 vendor 文件缺失）时静默跳过，不影响正文阅读。
function highlightCode(root) {
  if (!window.hljs) return;
  for (const block of root.querySelectorAll('pre code')) {
    // 超大代码块跳过高亮：hljs（尤其 auto-detect）对几十 KB 的块开销很大，
    // 是打开大文章时 CPU 尖峰的组成部分；纯文本展示不影响阅读。
    if ((block.textContent || '').length > 16000) continue;
    try {
      window.hljs.highlightElement(block);
    } catch {
      /* 检测失败/未知语言：原样显示 */
    }
  }
}

/// 应用主题三态：system 移除 data-theme（交给 CSS 媒体查询实时跟随），
/// light/dark 固定属性。非法值一律按 system 处理（与 Rust 侧白名单双保险）。
function applyTheme(pref) {
  const root = document.documentElement;
  if (pref === 'light' || pref === 'dark') {
    root.dataset.theme = pref;
  } else {
    delete root.dataset.theme;
  }
}

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
  folders: [],
  collapsedFolders: [],
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
  { kind: 'later', key: 'list.later', icon: '⏱' },
  { kind: 'all', key: 'list.all', icon: '≡' },
];

// ---------------------------------------------------------------- 正文清洗

const ALLOWED_TAGS = {
  p: [], br: [], hr: [], h1: [], h2: [], h3: [], h4: [], h5: [], h6: [],
  ul: [], ol: [], li: [], blockquote: [],
  table: [], thead: [], tbody: [], tfoot: [], tr: [], th: [], td: [],
  figure: [], figcaption: [], div: [], span: [],
  strong: [], em: [], b: [], i: [], u: [], s: [], del: [], ins: [], sup: [], sub: [], mark: [],
  img: ['src', 'alt', 'title', 'width', 'height'],
  a: ['href', 'title'],
  // class 仅放行 language-* token（语法高亮检测用），其余类名在属性循环里逐 token 剥除
  pre: ['class'],
  code: ['class'],
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
      // class 值逐 token 过滤：只保留 language-* 前缀，其余全部剥除；
      // 过滤后为空则整个移除（防任意类名注入）
      if (name === 'class') {
        const kept = value.split(/\s+/).filter((c) => /^language-[\w+#.-]+$/.test(c));
        if (kept.length) node.setAttribute('class', kept.join(' '));
        else node.removeAttribute('class');
        continue;
      }
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

// ---------------- 侧栏渲染：单一 keyed reconcile 路径 ----------------
// 所有触发方（阅读后的计数刷新/视图切换/文件夹管理/语言切换/搜索态）都只调
// renderSidebar() 这一个入口；入口内部按 key 复用已有行、只更新变化字段、
// 按期望顺序归位、清掉消失的行。不存在「全量重建」与「增量补丁」两条路径，
// 也就没有两套逻辑互相漂移的问题（这是选统一架构而非窄版补丁的原因）。
// #feeds 内的行不挂逐行监听器：点击/双击/右键由容器统一代理（initSidebarEvents），
// 事件发生时从 state 现查数据对象，行复用永远拿不到过期闭包。
// （#views 的行保留 onclick：绑定的是常量 VIEWS 项，无过期闭包风险。）

function setText(node, text) {
  // 文本没变就不写 DOM：阅读场景下每次刷新只有一两个数字在变
  if (node.textContent !== text) node.textContent = text;
}

/** 把容器子节点归位成 desired 的顺序：错位就移动，多余的删掉 */
function reconcileChildren(container, desired) {
  let cursor = container.firstChild;
  for (const node of desired) {
    if (node === cursor) {
      cursor = cursor.nextSibling;
      continue;
    }
    container.insertBefore(node, cursor || null);
  }
  while (cursor) {
    const next = cursor.nextSibling;
    cursor.remove();
    cursor = next;
  }
}

function reconcileViews(existing) {
  const counts = {
    unread: state.db ? state.db.unread : 0,
    starred: state.db ? state.db.starred : 0,
    later: state.db ? state.db.later : 0,
    all: state.db ? state.db.entries : 0,
  };
  const desired = VIEWS.map((v) => {
    const key = `v:${v.kind}`;
    let li = existing.get(key);
    if (!li) {
      li = document.createElement('li');
      li.dataset.key = key;
      li.dataset.kind = v.kind;
      li.innerHTML = `<span class="icon">${v.icon}</span><span class="vlabel"></span><span class="count"></span>`;
      li.onclick = () => setView({ kind: v.kind });
    }
    li.className = state.view.kind === v.kind ? 'active' : '';
    setText(li.querySelector('.vlabel'), t(v.key));
    setText(li.querySelector('.count'), String(counts[v.kind] ?? 0));
    return li;
  });
  reconcileChildren(el('views'), desired);
}

function feedRow(f, existing) {
  const key = `f:${f.id}`;
  let li = existing.get(key);
  if (!li) {
    li = document.createElement('li');
    li.dataset.key = key;
    li.dataset.feedId = String(f.id);
    li.innerHTML = '<span class="name"></span><span class="dot" hidden>●</span><span class="count"></span>';
  }
  const failed = !!(f.last_status && f.last_status !== 'ok' && f.last_status !== 'not_modified');
  li.className = `${state.view.kind === 'feed' && state.feedId === f.id ? 'active' : ''} folder-feed`;
  li.title = failed
    ? t('sidebar.feedTooltipFailed', {
        status: f.last_status,
        error: f.last_error || '',
      })
    : t('sidebar.feedTooltipOk', { url: f.url });
  setText(li.querySelector('.name'), f.title);
  li.querySelector('.dot').hidden = !failed;
  setText(li.querySelector('.count'), String(f.unread));
  return li;
}

function folderHead(folder, unreadSum, collapsed, existing) {
  const key = `h:${folder.id}`;
  let li = existing.get(key);
  if (!li) {
    li = document.createElement('li');
    li.dataset.key = key;
    li.dataset.folderId = String(folder.id);
    li.className = 'folder-head';
    li.innerHTML = '<span class="folder-arrow"></span><span class="name"></span><span class="count"></span>';
  }
  setText(li.querySelector('.folder-arrow'), collapsed ? '▸' : '▾');
  setText(li.querySelector('.name'), folder.name);
  setText(li.querySelector('.count'), unreadSum ? String(unreadSum) : '');
  return li;
}

function reconcileFeeds(existing) {
  // 分组在前（position 序），未分组垫底；组头含聚合未读数，点击折叠/展开
  const desired = [];
  for (const folder of state.folders || []) {
    const members = state.feeds.filter((f) => f.folder_id === folder.id);
    const collapsed = state.collapsedFolders.includes(folder.id);
    desired.push(folderHead(folder, members.reduce((acc, f) => acc + f.unread, 0), collapsed, existing));
    if (collapsed) continue;
    for (const f of members) desired.push(feedRow(f, existing));
  }
  for (const f of state.feeds.filter((f) => f.folder_id == null)) desired.push(feedRow(f, existing));
  reconcileChildren(el('feeds'), desired);
}

function renderSidebar() {
  const __st = performance.now();
  // key→既有行，一次性收集；后续 get-or-create 全靠它
  const collectRows = (id) => {
    const map = new Map();
    for (const li of el(id).children) {
      if (li.dataset.key) map.set(li.dataset.key, li);
    }
    return map;
  };
  reconcileViews(collectRows('views'));
  reconcileFeeds(collectRows('feeds'));
  setText(el('feeds-meta'), t('sidebar.feedCount', { n: state.feeds.length }));
  if (state.db) {
    const info = el('db-info');
    setText(info, `${state.db.entries} 篇 · ${state.db.dbPath}`);
    info.title = state.db.dbPath;
  }
  window.__SIDEBAR_MS = +(performance.now() - __st).toFixed(1);
  log(`renderSidebar feeds=${state.feeds.length} ${window.__SIDEBAR_MS}ms`);
}

/** #feeds 容器级事件代理：行复用不重挂监听，数据在事件时刻现查 */
function initSidebarEvents() {
  const feeds = el('feeds');
  feeds.addEventListener('click', (ev) => {
    const li = ev.target.closest('li');
    if (!li) return;
    if (li.classList.contains('folder-head')) {
      toggleFolderCollapse(Number(li.dataset.folderId));
      return;
    }
    if (li.dataset.feedId != null) setView({ kind: 'feed', feedId: Number(li.dataset.feedId) });
  });
  feeds.addEventListener('dblclick', (ev) => {
    const li = ev.target.closest('li[data-feed-id]');
    if (li) refreshOne(Number(li.dataset.feedId));
  });
  feeds.addEventListener('contextmenu', (ev) => {
    const head = ev.target.closest('li.folder-head');
    if (head) {
      ev.preventDefault();
      ev.stopPropagation();
      const folder = (state.folders || []).find((fo) => fo.id === Number(head.dataset.folderId));
      if (folder) openFolderMenu(ev, folder);
      return;
    }
    const li = ev.target.closest('li[data-feed-id]');
    if (li) {
      ev.preventDefault();
      const f = state.feeds.find((row) => row.id === Number(li.dataset.feedId));
      if (f) openFeedMenu(ev, f);
    }
  });
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
  const __t0 = performance.now();
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
    const laterMark = `<span class="later-mark ${e.read_later ? 'on' : ''}" data-later-id="${e.id}" title="${t('list.later')}">⚑</span>`;
    li.innerHTML = `
      <span class="title">${escapeHtml(e.title)}</span>
      <span class="meta"><span>${escapeHtml(e.feed_title)}</span><span>${fmtTime(e.published_at)}</span>${star}${laterMark}</span>
      ${e.summary ? `<span class="summary">${escapeHtml(e.summary)}</span>` : ''}`;
    li.onclick = () => openEntry(e.id, { markRead: true });
    const mark = li.querySelector('.later-mark');
    if (mark) {
      mark.onclick = (ev) => {
        ev.stopPropagation();
        state.selectedId = e.id;
        toggleReadLater().catch((err) => setStatus(err.message, true));
      };
    }
    list.appendChild(li);
  }
  window.__LIST_MS = +(performance.now() - __t0).toFixed(1);
  log(`renderList rows=${state.entries.length} ${window.__LIST_MS}ms`);
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

/** 未读视图：只移除一行 DOM。打开文章时避免 200 行全量重建的 CPU 尖峰
 *  （全量重建会把 CPU 打满，连并发后端命令都被拖慢一个量级：
 *   实测 get_entry 本体 0.07ms，撞上重建风暴时被拖到 60-150ms）。 */
function removeListRow(id) {
  const li = el('entries').querySelector(`li[data-id="${id}"]`);
  if (li) li.remove();
}

function renderReaderEmpty() {
  el('reader').innerHTML = `<div class="reader-empty">
      <p>${t('reader.empty')}</p>
      <p class="dim">${t('reader.shortcuts')}</p>
    </div>`;
}

function renderReader(entry) {
  const __t = [{ tag: 'start', ms: performance.now() }];
  const mark = (tag) => __t.push({ tag, ms: performance.now() });
  const reader = el('reader');
  const body = entry.content_html
    ? sanitize(entry.content_html, entry.url)
    : (entry.content_text || '')
        .split(/\n{1,}/)
        .map((p) => `<p>${escapeHtml(p)}</p>`)
        .join('');
  mark('sanitize');

  reader.innerHTML = `
    <div class="reader-head">
      <h1>${escapeHtml(entry.title)}</h1>
      <div class="meta">
        <span>${escapeHtml(entry.feed_title)}</span>
        <span>${fmtTime(entry.published_at)}</span>
        ${entry.author ? `<span>${escapeHtml(entry.author)}</span>` : ''}
      </div>
    </div>`;
  reader.insertAdjacentHTML('beforeend', `
    <div class="reader-actions">
      <button id="act-read">${entry.read ? t('reader.markUnread') : t('reader.markRead')}</button>
      <button id="act-star">${entry.starred ? t('reader.removeStar') : t('reader.addStar')}</button>
      <button id="act-later" class="${entry.read_later ? 'later-active' : ''}">${entry.read_later ? t('reader.removeLater') : t('reader.markLater')}</button>
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
    <div class="article">${body}</div>`);

  mark('innerHTML-set');
  // 高亮必须在正文插入 DOM 之后跑（hljs 需要真实节点）；
  // 输入是 sanitize 产物，hljs 输出不回灌 sanitize 流程。
  highlightCode(reader);
  mark('highlight');
  window.__RENDER_TIMINGS = __t.concat([{ tag: 'total', ms: +(performance.now() - __t[0].ms).toFixed(1) }]);
  // 打点外显：sanitize / DOM 写入 / 高亮三段耗时直接进终端日志（ui_log），
  // 不用开 devtools 就能定位卡顿在哪一段。
  {
    const t0 = __t[0].ms;
    const parts = __t.slice(1).map((x) => `${x.tag}=${Math.round(x.ms - t0)}`).join(' ');
    log(`renderReader id=${entry.id} ${parts} total=${Math.round(performance.now() - t0)}ms`);
  }

  el('act-read').onclick = () => toggleRead();
  el('act-star').onclick = () => toggleStar();
  el('act-later').onclick = () => toggleReadLater();
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

/// 全量读数据并渲染。
/// `reader: false` 是后台刷新用的静默模式：只重读侧栏与列表，不重渲染正文——
/// 阅读焦点与滚动位置保持原位（定时刷新到点时用户可能正在读一篇长文）。
async function loadAll({ reader = true } = {}) {
  const [sidebar, settings, ai, mcp, collapsed] = await Promise.all([
    invoke('sidebar_data'),
    invoke('get_ui_settings'),
    invoke('get_ai_settings'),
    invoke('get_mcp_settings'),
    invoke('get_collapsed_folders'),
  ]);
  state.db = sidebar.db;
  state.feeds = sidebar.feeds;
  state.folders = sidebar.folders;
  state.collapsedFolders = collapsed;
  state.settings = settings;
  state.ai = ai;
  state.mcp = mcp;
  // 语言设置来自数据库；先应用再渲染，避免先闪一下默认语言
  setLocale(settings.locale || 'auto');
  // 主题同理：渲染前先设好 data-theme，避免启动时闪错色
  applyTheme(settings.theme || 'system');
  applyStaticI18n();
  renderSidebar();
  await loadEntries({ reader });
  log(
    `loaded feeds=${sidebar.db.feeds} entries=${sidebar.db.entries} unread=${sidebar.db.unread} starred=${sidebar.db.starred} markReadOnNavigate=${settings.mark_read_on_navigate} refreshInterval=${settings.refresh_interval_minutes} refreshOnStart=${settings.refresh_on_start} ai=${ai.provider}${ai.model ? '/' + ai.model : '（未配模型）'} hasKey=${ai.has_key} mcp=${mcp.running ? mcp.url : 'off'}${reader ? '' : ' silent（正文未重渲染）'}`
  );
}

async function loadEntries({ reader = true } = {}) {
  const kind = state.view.kind;
  let rows;
  if (kind === 'search') {
    rows = state.query ? await invoke('search', { query: state.query, limit: 200 }) : [];
  } else {
    rows = await invoke('list_entries', {
      feedId: kind === 'feed' ? state.feedId : null,
      unreadOnly: kind === 'unread',
      starredOnly: kind === 'starred',
      readLaterOnly: kind === 'later',
      limit: 200,
    });
  }
  state.entries = rows;
  if (!rows.some((e) => e.id === state.selectedId)) {
    state.selectedId = rows.length ? rows[0].id : null;
  }
  renderList();
  // 静默模式到此为止：正文区一个 DOM 都不动
  if (!reader) return;
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
      // 未读视图里读过的文章会离开列表 → 只删那一行 DOM 并移动高亮，
      // 不做全量重建（200 行 renderList 是打开文章时的 CPU 尖峰来源）。
      // 右侧保持用户刚点开的文章（不再额外渲染 next，避免大文章连续两次
      // sanitize 造成可感卡顿）。
      const idx = state.entries.findIndex((e) => e.id === id);
      state.entries = state.entries.filter((e) => e.id !== id);
      const next = state.entries[Math.min(idx, state.entries.length - 1)];
      state.selectedId = next ? next.id : null;
      if (state.entries.length) {
        removeListRow(id);
        el('list-count').textContent = t('list.count', { n: state.entries.length });
      } else {
        renderList(); // 列表清空：走原路径渲染「暂无未读」占位
      }
      focusRow(state.selectedId, { follow: true });
      if (!next) renderReaderEmpty();
    } else {
      markRowRead(id);
    }
    // 计数刷新节流：连续快速阅读时合并为一次全量刷新（600ms 去抖）
    refreshCountsSoon();
  }
  log(`open id=${id} markRead=${markRead} read=${entry.read}`);
}

/// 计数刷新去抖：连续阅读时避免每次点击都全量拉取（db_info/list_feeds 聚合）。
/// 600ms 内的多次标记合并为一次；需要立即一致的路径（视图切换/手动刷新）仍可直调 refreshCounts。
let refreshCountsTimer = null;
function refreshCountsSoon() {
  if (refreshCountsTimer) return;
  refreshCountsTimer = setTimeout(async () => {
    refreshCountsTimer = null;
    try {
      await refreshCounts();
    } catch (e) {
      log(`refreshCounts failed: ${e.message}`);
    }
  }, 600);
}

async function refreshCounts() {
  // 单命令单次锁：避免 db_info/list_feeds/list_folders 三命令并发抢 Mutex 排队
  const data = await invoke('sidebar_data');
  state.db = data.db;
  state.feeds = data.feeds;
  state.folders = data.folders;
  renderSidebar();
}

// ---------------- 侧栏文件夹：折叠与右键管理 ----------------

function toggleFolderCollapse(folderId) {
  const idx = state.collapsedFolders.indexOf(folderId);
  if (idx >= 0) state.collapsedFolders.splice(idx, 1);
  else state.collapsedFolders.push(folderId);
  invoke('set_collapsed_folders', { ids: [...state.collapsedFolders] }).catch(() => {});
  renderSidebar();
}

function closeContextMenu() {
  el('ctx-menu')?.remove();
}

/// 通用右键菜单：items = [{label, danger?, action}]
function openContextMenu(ev, items) {
  closeContextMenu();
  const menu = document.createElement('div');
  menu.id = 'ctx-menu';
  for (const item of items) {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.textContent = item.label;
    if (item.danger) btn.classList.add('danger');
    btn.onclick = () => {
      closeContextMenu();
      item.action();
    };
    menu.appendChild(btn);
  }
  document.body.appendChild(menu);
  const pad = 8;
  menu.style.left = Math.min(ev.clientX, window.innerWidth - menu.offsetWidth - pad) + 'px';
  menu.style.top = Math.min(ev.clientY, window.innerHeight - menu.offsetHeight - pad) + 'px';
}

function openFeedMenu(ev, feed) {
  const items = [];
  for (const folder of state.folders) {
    if (folder.id === feed.folder_id) continue;
    items.push({
      label: t('menu.moveToWithName', { name: folder.name }),
      action: () => reassignFeed(feed.id, folder.id),
    });
  }
  if (feed.folder_id != null) {
    items.push({ label: t('menu.moveToUngrouped'), action: () => reassignFeed(feed.id, null) });
  }
  if (!items.length) return;
  openContextMenu(ev, items);
}

function openFolderMenu(ev, folder) {
  openContextMenu(ev, [
    { label: t('menu.rename'), action: () => renameFolder(folder) },
    { label: t('menu.delete'), danger: true, action: () => deleteFolder(folder) },
  ]);
}

function reassignFeed(feedId, folderId) {
  invoke('assign_feed_folder', { feedId, folderId })
    .then(() => refreshCounts())
    .catch((err) => setStatus(err.message, true));
}

/// 内联文本输入对话框（Tauri 禁用 window.prompt）：resolve(null)=取消
function promptText(title, initial) {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'prompt-overlay';
    overlay.innerHTML = `
      <div class="prompt-box">
        <div class="prompt-title">${escapeHtml(title)}</div>
        <input type="text" autocomplete="off" />
        <div class="prompt-actions">
          <button type="button" data-act="cancel"></button>
          <button type="button" data-act="ok" class="primary"></button>
        </div>
      </div>`;
    const input = overlay.querySelector('input');
    const cancelBtn = overlay.querySelector('[data-act="cancel"]');
    const okBtn = overlay.querySelector('[data-act="ok"]');
    cancelBtn.textContent = t('prompt.cancel');
    okBtn.textContent = t('prompt.ok');
    const done = (value) => {
      overlay.remove();
      document.removeEventListener('keydown', onKey, true);
      resolve(value);
    };
    const onKey = (e) => {
      if (e.key === 'Escape') done(null);
      if (e.key === 'Enter') done(input.value.trim() || null);
    };
    document.addEventListener('keydown', onKey, true);
    okBtn.onclick = () => done(input.value.trim() || null);
    cancelBtn.onclick = () => done(null);
    overlay.onclick = (e) => { if (e.target === overlay) done(null); };
    document.body.appendChild(overlay);
    input.value = initial || '';
    input.focus();
    input.select();
  });
}

/// 内联确认对话框：resolve(true)=确定，resolve(false)=取消
function confirmBox(message) {
  return new Promise((resolve) => {
    const overlay = document.createElement('div');
    overlay.className = 'prompt-overlay';
    overlay.innerHTML = `
      <div class="prompt-box">
        <div class="prompt-title">${escapeHtml(message)}</div>
        <div class="prompt-actions">
          <button type="button" data-act="cancel"></button>
          <button type="button" data-act="ok" class="primary"></button>
        </div>
      </div>`;
    const cancelBtn = overlay.querySelector('[data-act="cancel"]');
    const okBtn = overlay.querySelector('[data-act="ok"]');
    cancelBtn.textContent = t('prompt.cancel');
    okBtn.textContent = t('prompt.ok');
    const done = (value) => {
      overlay.remove();
      document.removeEventListener('keydown', onKey, true);
      resolve(value);
    };
    const onKey = (e) => {
      if (e.key === 'Escape') done(false);
      if (e.key === 'Enter') done(true);
    };
    document.addEventListener('keydown', onKey, true);
    okBtn.onclick = () => done(true);
    cancelBtn.onclick = () => done(false);
    overlay.onclick = (e) => { if (e.target === overlay) done(false); };
    document.body.appendChild(overlay);
    okBtn.focus();
  });
}

/// 新建分组入口（侧栏「订阅源」标题旁的 + 按钮）
async function createFolder() {
  const name = await promptText(t('folder.newTitle'), '');
  if (!name) return;
  try {
    await invoke('add_folder', { name });
    await refreshCounts();
    log(`folder added: ${name}`);
  } catch (err) {
    setStatus(err.message, true);
  }
}

async function renameFolder(folder) {
  const name = await promptText(t('folder.renameTitle'), folder.name);
  if (!name || name === folder.name) return;
  try {
    await invoke('rename_folder', { folderId: folder.id, name });
    await refreshCounts();
  } catch (err) {
    setStatus(err.message, true);
  }
}

async function deleteFolder(folder) {
  try {
    await invoke('delete_folder', { folderId: folder.id });
    await refreshCounts();
    log(`folder deleted: ${folder.name}`);
  } catch (err) {
    setStatus(err.message, true);
  }
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

/// 稍后读：与已读/星标独立；当前视图是稍后读时，取消标记要从列表移除该行
async function toggleReadLater() {
  const row = state.entries.find((e) => e.id === state.selectedId);
  if (!row) return;
  const readLater = !row.read_later;
  await invoke('set_read_later', { ids: [row.id], readLater });
  row.read_later = readLater;
  const fresh = await invoke('get_entry', { id: row.id });
  if (fresh) renderReader(fresh);
  if (state.view.kind === 'later' && !readLater) {
    await loadAll();
  } else {
    renderList();
    await refreshCounts();
  }
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

/// OPML 导入后只抓新导入的那批源（后端 refresh_feeds 与手动刷新共用同一条管线）。
/// 单 flight 挡下时给出可读提示，不静默吞掉。
async function fetchImportedFeeds(feedIds) {
  setStatus(t('status.importFetching', { n: feedIds.length }));
  log(`refresh_feeds imported=${feedIds.length}`);
  try {
    const r = await invoke('refresh_feeds', { feedIds, concurrency: 6 });
    setStatus(
      t('status.refreshDone', {
        fetched: r.fetched,
        notModified: r.not_modified,
        inserted: r.inserted,
        failures: r.failures.length,
      })
    );
    for (const f of r.failures) log(`import refresh failure feed=${f.feed_id} ${f.url} :: ${f.error}`);
  } catch (e) {
    setStatus(t('status.refreshFailed', { error: e.message }), true);
    log(`refresh_feeds failed: ${e.message}`);
  }
  await loadAll();
}

/// 后台刷新提示的原文（null = 没在显示）。只在状态栏还写着这句提示时才清除，
/// 避免把用户这一瞬间刚触发的文案（例如手动刷新被单 flight 拒绝的错误）抹掉。
let backgroundRefreshHint = null;

/// 后台刷新（定时 / 启动首刷）由 Rust 侧 emit `refresh:start` / `refresh:done`。
/// 手动刷新是同步等待且不发事件，所以这里只会收到后台刷新，不会出现双提示。
/// done 走静默 loadAll：侧栏与列表刷新，正文不重渲染，阅读焦点与滚动位置保持原位。
function initRefreshEvents() {
  const events = window.__TAURI__ && window.__TAURI__.event;
  if (!events || !events.listen) {
    // 事件 API 拿不到时不静默：后台刷新照旧跑，只是界面不会自动更新
    log('refresh 事件不可用（window.__TAURI__.event 缺失）：后台刷新不会自动更新界面');
    return;
  }
  events
    .listen('refresh:start', () => {
      log('refresh:start');
      backgroundRefreshHint = t('status.autoRefreshing');
      setStatus(backgroundRefreshHint);
    })
    .catch((e) => log(`listen refresh:start failed: ${e.message}`));
  events
    .listen('refresh:done', async () => {
      log('refresh:done');
      const hint = backgroundRefreshHint;
      backgroundRefreshHint = null;
      if (hint && el('status').textContent === hint) setStatus('');
      // 「静默」的定义就是正文 DOM 与阅读滚动位置一个字节都不动：把这条不变量打进日志，
      // 无人值守时也能核对（不用只能盯着屏幕看）
      const reader = el('reader');
      const idBefore = state.selectedId;
      const scrollBefore = reader.scrollTop;
      const htmlBefore = reader.innerHTML.length;
      try {
        await loadAll({ reader: false });
      } catch (e) {
        log(`refresh:done reload failed: ${e.message}`);
      }
      log(
        `refresh:done 静默完成 selected=${idBefore}→${state.selectedId} 正文=${htmlBefore}→${reader.innerHTML.length}字 scrollTop=${scrollBefore}→${reader.scrollTop}`
      );
    })
    .catch((e) => log(`listen refresh:done failed: ${e.message}`));
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
    <iframe src="https://evil.example"></iframe>
    <pre><code class="language-rust">fn main() {}</code></pre>
    <code class="evil-class another">rm -rf</code>
    <code class="language-py evil-x">print(1)</code></div>`;
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
  // class 白名单：language-* 保留，任意类名剥除，混合时逐 token 过滤
  if (!/class="language-rust"/.test(clean)) failures.push('code 的 language-* class 被误剥');
  if (/evil-class|another/.test(clean)) failures.push('非 language-* 类名残留');
  if (/class="[^"]*evil-x/.test(clean)) failures.push('混合 class 中非 language-* token 未剥除');
  if (!/class="language-py"/.test(clean)) failures.push('混合 class 中 language-* token 未保留');

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
  el('set-theme').value = state.settings.theme || 'system';
  el('set-close-action').value = state.settings.close_action || 'exit';
  el('set-refresh-interval').value = state.settings.refresh_interval_minutes || '30';
  el('set-refresh-on-start').checked = !!state.settings.refresh_on_start;
  el('set-rsshub-mirror').value = state.settings.rsshub_mirror || '';
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
  // 点击右键菜单以外的区域时关闭菜单（菜单内部点击不受影响）
  document.addEventListener('click', (e) => {
    if (el('ctx-menu') && !e.target.closest('#ctx-menu')) closeContextMenu();
  });
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
    return; // finally 仍会执行：初始化失败也要把窗口亮出来（错误状态 UI）
  } finally {
    // 窗口以 hidden 创建（防主题闪变）：主题/首屏就绪后显示；
    // 真正的显示动作在 Rust 侧，失败时由 5s 兑底定时器接管。
    invoke('show_main_window').catch(() => {});
  }

  el('btn-refresh').onclick = doRefresh;
  initRefreshEvents();
  initSidebarEvents();
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
  el('set-theme').addEventListener('change', async (e) => {
    try {
      state.settings = await invoke('set_ui_theme', { theme: e.target.value });
      applyTheme(state.settings.theme || 'system');
      log(`theme=${state.settings.theme}`);
    } catch (err) {
      setStatus(t('status.settingFailed', { error: err.message }), true);
    }
  });
  el('set-close-action').addEventListener('change', async (e) => {
    try {
      state.settings = await invoke('set_ui_close_action', { action: e.target.value });
      log(`closeAction=${state.settings.close_action}`);
    } catch (err) {
      setStatus(t('status.settingFailed', { error: err.message }), true);
    }
  });
  // 自绘标题栏三键（窗口无系统装饰）
  el('btn-new-folder').onclick = () => createFolder();
  el('btn-win-min').onclick = () => invoke('window_minimize').catch(() => {});
  el('btn-win-max').onclick = () => invoke('window_toggle_maximize').catch(() => {});
  el('btn-win-close').onclick = () => invoke('window_close').catch((e) => {
    setStatus(t('status.settingFailed', { error: e.message }), true);
  });
  // RSSHub 设置
  el('btn-rsshub-save').onclick = async () => {
    try {
      const mirror = await invoke('set_rsshub_mirror', { mirror: el('set-rsshub-mirror').value });
      state.settings.rsshub_mirror = mirror;
      el('rsshub-status').textContent = t('settings.rsshub.saved', { url: mirror });
      log(`rsshub mirror=${mirror}`);
    } catch (err) {
      el('rsshub-status').textContent = err.message;
    }
  };
  el('btn-rsshub-test').onclick = async () => {
    el('rsshub-status').textContent = t('settings.rsshub.testing');
    try {
      el('rsshub-status').textContent = await invoke('test_rsshub_mirror', {
        mirror: el('set-rsshub-mirror').value,
      });
    } catch (err) {
      el('rsshub-status').textContent = err.message;
    }
  };
  el('btn-rsshub-migrate').onclick = async () => {
    try {
      const hit = await invoke('preview_rsshub_migration');
      if (!hit) {
        el('rsshub-status').textContent = t('settings.rsshub.migrateNone');
        return;
      }
      const okToGo = await confirmBox(t('settings.rsshub.migrateConfirm', { n: String(hit) }));
      if (!okToGo) return;
      const out = await invoke('migrate_rsshub_feeds');
      if (out.errors && out.errors.length) {
        el('rsshub-status').textContent = t('settings.rsshub.migrateErrors', {
          n: String(out.migrated),
          error: out.errors[0],
        });
      } else if (out.skipped) {
        el('rsshub-status').textContent = t('settings.rsshub.migrateDoneWithSkipped', {
          m: String(out.migrated),
          s: String(out.skipped),
        });
      } else {
        el('rsshub-status').textContent = t('settings.rsshub.migrateDone', { n: String(out.migrated) });
      }
      log(`rsshub migration: migrated=${out.migrated} skipped=${out.skipped} errors=${out.errors.length}`);
      await refreshCounts();
    } catch (err) {
      el('rsshub-status').textContent = err.message;
    }
  };
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
  el('set-refresh-interval').addEventListener('change', async (e) => {
    try {
      state.settings = await invoke('set_refresh_interval', { minutes: e.target.value });
      // 后端是白名单唯一来源：把归一化后的值回显到控件，界面与库里不会各说一套
      const minutes = state.settings.refresh_interval_minutes;
      e.target.value = minutes;
      setStatus(
        minutes === 'off'
          ? t('status.refreshIntervalOff')
          : t('status.refreshIntervalSet', { minutes })
      );
      log(`refreshInterval=${minutes}`);
    } catch (err) {
      setStatus(t('status.settingFailed', { error: err.message }), true);
      log(`set_refresh_interval failed: ${err.message}`);
    }
  });
  el('set-refresh-on-start').addEventListener('change', async (e) => {
    try {
      state.settings = await invoke('set_refresh_on_start', { enabled: e.target.checked });
      e.target.checked = state.settings.refresh_on_start;
      setStatus(
        t(state.settings.refresh_on_start ? 'status.refreshOnStartOn' : 'status.refreshOnStartOff')
      );
      log(`refreshOnStart=${state.settings.refresh_on_start}`);
    } catch (err) {
      setStatus(t('status.settingFailed', { error: err.message }), true);
      log(`set_refresh_on_start failed: ${err.message}`);
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
      // 新导入的源在库里是 0 条目：立刻只抓这一批，省掉「导入完再手动刷一次」的断层。
      // 重复导入（新增 0）时不会白刷一遍全量。
      const newIds = r.added_feed_ids || [];
      if (r.feeds_added > 0 && newIds.length) await fetchImportedFeeds(newIds);
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

    if (e.key === 'Escape' && el('ctx-menu')) {
      closeContextMenu();
      return;
    }
    switch (e.key) {
      case 'j': case 'ArrowDown': e.preventDefault(); move(1); break;
      case 'k': case 'ArrowUp': e.preventDefault(); move(-1); break;
      case 'Enter': e.preventDefault(); if (state.selectedId) openEntry(state.selectedId, { markRead: true }); break;
      case 'u': e.preventDefault(); toggleRead().catch((err) => setStatus(err.message, true)); break;
      case 's': e.preventDefault(); toggleStar().catch((err) => setStatus(err.message, true)); break;
      case 'l': e.preventDefault(); toggleReadLater().catch((err) => setStatus(err.message, true)); break;
      case 'r': e.preventDefault(); doRefresh(); break;
      case 'g': e.preventDefault(); jump(false); break;
      case 'G': e.preventDefault(); jump(true); break;
      default: break;
    }
  });
}

window.addEventListener('DOMContentLoaded', boot);

})();
