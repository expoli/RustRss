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
    // 超大代码块跳过 auto-detect：hljs 对几十 KB 的块开销很大，是打开大文章时
    // CPU 尖峰的组成部分。已显式标注 language-diff 的块走的是行级正则、代价线性，
    // 放宽到 64KB（内核补丁动辄几十 KB，不能因此退化成无高亮）。
    const len = (block.textContent || '').length;
    const explicitDiff = block.classList.contains('language-diff');
    if (len > (explicitDiff ? 65536 : 16000)) continue;
    try {
      window.hljs.highlightElement(block);
      if (explicitDiff) flattenDiffBlockSpacing(block);
    } catch {
      /* 检测失败/未知语言：原样显示 */
    }
  }
}

/// hljs 的 diff 输出里，块级行 span（addition/deletion）之间的裸换行文本节点
/// 会各自占一整行（pre 保留白空格）——实测每条着色行后多出一行空白，补丁
/// 双倍行距。高亮后把「纯换行」节点删掉即可；带内容的上下文行节点保留。
function flattenDiffBlockSpacing(code) {
  for (const node of [...code.childNodes]) {
    if (node.nodeType === 3 && /^[\n\r]+$/.test(node.textContent || '')) {
      node.remove();
    }
  }
}

/// diff 行分类：返回 'add' | 'del' | 'hunk' | 'head' | null。
/// 注意真实 diff 的 +/− 后面直接跟代码（不一定是空格），所以只认前缀字符；
/// 误报（如破折号开头的段落）靠调用方的运行长度与混合签名门槛拦。
/// 邮件签名分隔行 `-- ` 单独排除：它以 - 开头但不是删除行（实测 lkml 文末
/// 被误标红过）。
function diffLineKind(line) {
  if (line.startsWith('@@')) return 'hunk';
  if (
    line.startsWith('diff --git ') ||
    line.startsWith('index ') ||
    line.startsWith('--- ') ||
    line.startsWith('+++ ') ||
    line.startsWith('new file mode ') ||
    line.startsWith('deleted file mode ')
  ) {
    return 'head';
  }
  if (line === '--' || line === '-- ') return null;
  if (line.startsWith('+')) return 'add';
  if (line.startsWith('-')) return 'del';
  return null;
}

/// 合并门槛：marked>=4 且（有 hunk/头行且增删行合计≥2，或增删都存在）。
/// 第二个分支覆盖真实邮件补丁：lkml 的 cover-letter/纯删补丁可能一条 + 行
/// 都没有（实测 2026-09-22：2 个 @@ + 2 个 - 行的补丁被旧门槛拒之门外）；
/// @@ 在散文里几乎不存在，所以 hunkOrHead 分支的误报风险足够低。
function isStrongDiffSignature(marked, adds, dels, hunksOrHeads) {
  if (marked < 4) return false;
  if (hunksOrHeads >= 1 && adds + dels >= 2) return true;
  return adds >= 1 && dels >= 1;
}

/// lkml 等邮件列表源把补丁拆成一连串 <p>（每行一段）——没有 pre/code，
/// 高亮管线（`pre code`）根本匹配不到，补丁就以比例字体正文样式渲染
/// （2026-09-22 截图实测：无等宽、无底色、无 +/- 配色，上下文行缩进还被
/// HTML 塌掉）。sanitize 之后、highlightCode 之前做一次「diff 区域归一」：
/// 把连续 diff 形状段落合并成单个 <pre><code class="language-diff">。
/// 保守门槛：至少 4 个标记行（+/-/@@/头行），且同时含 + 行与（- 行或 @@/头行），
/// 避免把普通列表/破折号段落误吞。
function normalizeDiffBlocks(root) {
  const blocks = [...root.children];
  const kinds = blocks.map((b) => {
    if (b.tagName === 'PRE') return 'pre';
    const t = (b.textContent || '').trim();
    return t === '' ? 'blank' : diffLineKind(t);
  });

  // 预筛（廉价必要条件）：要么出现 hunk/头行（完整补丁形态），要么 +/- 行
  // 都存在（截断片段形态——lkml 摘要把 @@ 头切掉只留 +/- 行，实测 2026-09-22
  // k3-udma 补丁摘要 11- / 7+ 行被旧预筛整体跳过，无配色无等宽）；含 <pre> 的
  // 正文交给第二阶段逐块判定，不在预筛里下结论。预筛成本与旧版同阶（children
  // 级而非 textContent 级，多数文章零分配）。
  let cheapHunk = false, cheapAdds = 0, cheapDels = 0, cheapPre = false;
  for (const k of kinds) {
    if (k === 'pre') { cheapPre = true; continue; }
    if (k === 'hunk' || k === 'head') { cheapHunk = true; break; }
    if (k === 'add') cheapAdds++;
    else if (k === 'del') cheapDels++;
  }
  if (!cheapPre && !cheapHunk && !(cheapAdds >= 1 && cheapDels >= 1)) return;

  let i = 0;
  while (i < blocks.length) {
    // 找一段连续的可疑区（标记行/空行，且首行必须是标记行）
    if (!kinds[i] || kinds[i] === 'pre' || kinds[i] === 'blank') { i++; continue; }
    let j = i;
    let marked = 0, adds = 0, dels = 0, hunksOrHeads = 0;
    const scan = () => {
      marked = adds = dels = hunksOrHeads = 0;
      for (let k = i; k <= j; k++) {
        const kind = kinds[k];
        if (kind === 'add') { marked++; adds++; }
        else if (kind === 'del') { marked++; dels++; }
        else if (kind === 'hunk' || kind === 'head') { marked++; hunksOrHeads++; }
      }
    };
    // 区域延伸：后面跟着的标记行、空行、以及「短的单行上下文段」（补丁上下文行
    // 缩进被塌掉后与普通段落无异，只吸收短行，遇到长句/散文即停）
    while (j + 1 < blocks.length) {
      const nk = kinds[j + 1];
      const nb = blocks[j + 1];
      const nt = (nb.textContent || '').trim();
      const isPlainShortLine = !nk && nk !== 'pre' && nb.tagName !== 'PRE' && nt.length > 0 && nt.length <= 120;
      if (nk === 'blank' || (nk && nk !== 'pre') || isPlainShortLine) j++;
      else break;
    }
    scan();
    if (isStrongDiffSignature(marked, adds, dels, hunksOrHeads)) {
      // 收集行文本（含被吸收的上下文/空行），去掉尾部空行
      const lines = [];
      for (let k = i; k <= j; k++) {
        const t = blocks[k].textContent || '';
        t.split('\n').forEach((l) => lines.push(l.replace(/\s+$/, '')));
      }
      while (lines.length && lines[lines.length - 1] === '') lines.pop();
      if (lines.join('\n').length <= 200000) {
        const pre = document.createElement('pre');
        const code = document.createElement('code');
        code.className = 'language-diff';
        code.textContent = lines.join('\n');
        pre.appendChild(code);
        blocks[i].replaceWith(pre);
        for (let k = i + 1; k <= j; k++) blocks[k].remove();
      }
    }
    i = j + 1;
  }

  // 第二类结构：源直接把补丁放进 <pre>（无 language 类）——上面段落归一
  // 刻意跳过 pre，这里补上：对无 language-*/lang-* 类的代码块做同样的形状探测，
  // 强签名就补 language-diff（否则交给 hljs auto-detect 不可靠：大块常被
  // 跳过或选错语言）。裸 <pre>（无 code 子元素）强签名时包一层 code。
  for (const pre of root.querySelectorAll('pre')) {
    let code = pre.querySelector(':scope > code');
    if (code && (code.className.includes('language-') || code.className.includes('lang-'))) continue;
    const text = (code || pre).textContent || '';
    const lines = text.split('\n');
    let marked = 0, adds = 0, dels = 0, hunksOrHeads = 0;
    for (const l of lines) {
      const k = diffLineKind(l.trim());
      if (k === 'add') { marked++; adds++; }
      else if (k === 'del') { marked++; dels++; }
      else if (k === 'hunk' || k === 'head') { marked++; hunksOrHeads++; }
    }
    if (isStrongDiffSignature(marked, adds, dels, hunksOrHeads) === false) continue;
    if (!code) {
      code = document.createElement('code');
      code.textContent = text;
      pre.textContent = '';
      pre.appendChild(code);
    }
    code.classList.add('language-diff');
  }
}

/// 应用 core 解析后的主题快照；系统模式由共享渲染器监听媒体查询。
let themeRenderer;
function applyTheme() {
  const snapshot = state.settings.theme_snapshot;
  if (!snapshot) return;
  themeRenderer ||= window.RustRssTheme.createRenderer(document.documentElement, { reader: el('reader'), list: el('entries'), onModeChange: refreshSettingDropdowns });
  themeRenderer.apply(snapshot);
}

// ---------------------------------------------------------------- 字体配置

/// 正文字号 / 行高的区间、步长与默认值：与 Rust 侧 clamp 区间同源
/// （HTML 的 min/max/step 只是静态兜底，真正的判据在这里 + Rust）。
const FONT_SIZE = { min: 13, max: 28, step: 1, fallback: 14 };
const FONT_LINE = { min: 1.3, max: 2.2, step: 0.05, fallback: 1.55 };

/// 数值兜底：非有限值回默认，越界夹回区间（库里被写坏也不让排版崩）
function clampNumber(value, { min, max, fallback }) {
  const n = Number(value);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, n));
}

/// 数值显示口径：字号取整、行高两位小数（与落库口径一致，标签不跳字）
const fontSizeText = (v) => String(clampNumber(v, FONT_SIZE));
const fontLineText = (v) => clampNumber(v, FONT_LINE).toFixed(2);

function acceptSettings(settings) {
  const current = state.settings?.theme_snapshot;
  if (current && settings.theme_snapshot?.config.revision < current.config.revision) {
    settings = { ...settings, theme_snapshot: current, theme: state.settings.theme,
      font_ui: state.settings.font_ui, font_read: state.settings.font_read, font_mono: state.settings.font_mono,
      font_read_size: state.settings.font_read_size, font_read_line: state.settings.font_read_line };
  }
  state.settings = settings;
}

function acceptThemeSnapshot(snapshot) {
  const config = snapshot.config;
  if (config.revision < (state.settings.theme_snapshot?.config.revision ?? -1)) return;
  const overrides = config.overrides.typography || {};
  const dark = config.mode === 'dark' || (config.mode === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
  const typography = snapshot[dark ? 'dark' : 'light'].typography;
  acceptSettings({ ...state.settings, theme_snapshot: snapshot, theme: config.mode,
    font_ui: overrides.ui_family?.[0] || '', font_read: overrides.read_family?.[0] || '', font_mono: overrides.mono_family?.[0] || '',
    font_read_size: typography.read_size, font_read_line: typography.line_height });
  applyTheme();
  refreshSettingDropdowns();
  themeEditors.forEach(e => e.refresh()); aaEditor?.refresh();
  log(`theme applied revision=${config.revision} hash=${snapshot.config_hash}`);
}

async function startThemeSync() {
  const sync = window.RustRssThemeSync.createSync({
    read: knownRevision => invoke('get_theme_update', { knownRevision }),
    revision: () => state.settings.theme_snapshot?.config.revision ?? null,
    apply: acceptThemeSnapshot,
    active: () => !document.hidden && document.hasFocus(),
    onError: error => log(`theme sync failed: ${error.message}`),
  });
  // Subscribe before initial settings read; focus/polling cover independent stdio writes.
  let unlisten;
  try { unlisten = await window.__TAURI__?.event?.listen('theme:changed', () => sync.check(true)); }
  catch (error) { log(`theme event unavailable; polling remains active: ${error.message}`); }
  const focus = () => sync.check();
  window.addEventListener('focus', focus);
  document.addEventListener('visibilitychange', focus);
  const timer = setInterval(() => sync.check(), 2000);
  window.addEventListener('pagehide', () => {
    clearInterval(timer); sync.dispose(); unlisten?.();
    window.removeEventListener('focus', focus); document.removeEventListener('visibilitychange', focus);
  }, { once: true });
}

async function invoke(cmd, args = {}) {
  if (!window.__TAURI__ || !window.__TAURI__.core) {
    throw new Error('IPC 不可用（不在 Tauri 中运行？）');
  }
  try {
    return await window.__TAURI__.core.invoke(cmd, args);
  } catch (e) {
    const error = new Error(typeof e === 'string' ? e : (e && e.message ? e.message : String(e)));
    if (e && typeof e.code === 'string') error.code = e.code;
    throw error;
  }
}

/** 诊断输出：打到应用 stdout，便于无人值守时核对界面状态（不依赖肉眼看屏幕） */
function log(line) {
  console.log('[ui]', line);
  invoke('ui_log', { line }).catch(() => {});
}

// CSP 违规探针：常驻监听，被 CSP 拦下的指令逐条上报（含被拒指令与来源）。
// 装它的理由：CSP 是纵深防线，静默生效就没有验收手段——配置漏放行某个资源类型时，
// 应该是日志里的一行（可机械核对「零违规」），而不是「图片莫名其妙不显示」。
// 注：本文件在 body 末尾加载，此前解析期的违规（如外链脚本被拒）由 index.html
// 的内联探针（script load error / js error）覆盖。
document.addEventListener('securitypolicyviolation', (e) => {
  log(
    `securitypolicyviolation: ${e.violatedDirective} blocked=${e.blockedURI || '-'} ` +
      `source=${e.sourceFile || '-'}:${e.lineNumber || 0}`
  );
});

const state = {
  db: null,
  feeds: [],
  folders: [],
  collapsedFolders: [],
  entries: [],
  // 会话内已读的条目 id：未读视图里读过一行就从列表删掉，后台刷新 prepend 时不能再把它
  // 插回来（服务端 unread 过滤在竞态下会漏——刷新的查询可能先于 set_read 提交）。
  readSessionIds: new Set(),
  view: { kind: 'unread' },
  feedId: null,
  selectedId: null,
  // 阅读区当前展示的条目对象（就是 renderReader 的入参）。它与列表里的行是**两个**
  // 对象（list_entries 的行 / get_entry 的详情），行还可能因当前视图的过滤离开列表
  // 而正文仍在屏幕上——标记翻转要找得到「用户正在看的那篇」，见 toggleTarget。
  readerEntry: null,
  // 正文区当前展示条目所属的源 id：改源名后只更新该源的元信息（正文不重渲染）
  readerFeedId: null,
  query: '',
  // 权威值在 Rust 侧（get_ui_settings），这里只是启动前的占位
  settings: { mark_read_on_navigate: true },
  // 系统字体族（设置页打开时预取一次并缓存；空 = 未取到/非 Linux → 只剩「跟随主题」）
  fontFamilies: [],
  fontFamiliesLoaded: false,
  ai: null,
  mcp: null,
  // 标签缓存（core 的 TagRow 原样，含未读计数/颜色/`last_used_at`）：chips 渲染、
  // 选择器与标签视图标题共用一份；打标后调 refreshTagCache() 整体刷新（最近使用
  // 顺序与计数都在里面）。chips 自身的来源是 EntryRow.tags，不从这里反查。
  tags: [],
  // 侧栏「标签」区的行：同一张表，**core 的侧栏口径**（置顶优先 → 手动顺序 →
  // 名称，`sidebar_data.tags` 带回）。与 `tags`（选择器口径：最近使用优先）并存
  // 是刻意的——两个 ORDER BY 都在 core，前端不重排、不数数。
  sidebarTags: [],
  // 标签区折叠状态（跨会话保持在设置 `ui.tags_collapsed` 里，与文件夹折叠同模式）
  tagsCollapsed: false,
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

function bindSidebarKeyboard(row) {
  row.tabIndex = 0;
  row.setAttribute('role', 'button');
  row.addEventListener('keydown', e => {
    if (e.target !== row || !['Enter', ' '].includes(e.key)) return;
    e.preventDefault();
    e.stopPropagation();
    row.click();
  });
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
      bindSidebarKeyboard(li);
      li.innerHTML = window.RustRssComponents.viewContent(v.icon);
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
    bindSidebarKeyboard(li);
    li.draggable = true;
    li.innerHTML = window.RustRssComponents.feedContent();
  }
  const failed = !!(f.last_status && f.last_status !== 'ok' && f.last_status !== 'not_modified');
  li.className = `${state.view.kind === 'feed' && state.feedId === f.id ? 'active' : ''} folder-feed`;
  const tooltip = failed
    ? t('sidebar.feedTooltipFailed', {
        status: f.last_status,
        error: fetchFailureMessage(f.last_status, f.last_error || ''),
      })
    : t('sidebar.feedTooltipOk', { url: f.url });
  // 独立档位是「看不见的设置」：跟随全局的源不写这一行，有覆盖的源在常驻
  // tooltip 里显出来（否则只有打开右键菜单才知道这个源跟别人不一样）。
  li.title =
    f.refresh_interval_minutes == null
      ? tooltip
      : `${tooltip}\n${t('sidebar.feedTooltipInterval', {
          interval: feedIntervalLabel(f.refresh_interval_minutes),
        })}`;
  setText(li.querySelector('.name'), f.title);
  li.querySelector('.dot').hidden = !failed;
  setText(li.querySelector('.count'), String(f.unread));
  return li;
}

function fetchFailureMessage(code, fallback = '') {
  const keys = { retry_deferred: 'deferred', timeout: 'timeout', connection_error: 'connection', network_error: 'network',
    redirect_error: 'redirect', invalid_url: 'invalidUrl', too_large: 'tooLarge',
    body_error: 'body', parse_error: 'parse', no_feed_link: 'noFeed', unexpected_response: 'response',
    // 代理族：core 的 network.rs 会以这五个码回失败，不映射就会落到 fetchError.unknown，
    // 用户看不出「是代理配置坏了」。码集合由 scripts/tests/fetch-error-mapping.test.cjs 守着。
    proxy_client_lock: 'proxyLock', proxy_client_setup: 'proxySetup', proxy_invalid_url: 'proxyUrl',
    proxy_invalid_config: 'proxyConfig', proxy_credentials_not_supported: 'proxyCredentials' };
  if (code === 'http_429') return t('fetchError.rateLimited');
  if (/^http_[45]\d\d$/.test(code || '')) return t('fetchError.http', { status: code.slice(5) });
  // Older versions stored response-body failures as http_2xx. Do not describe
  // these as server rejections or inspect translated diagnostic strings.
  if (/^http_2\d\d$/.test(code || '')) return t('fetchError.body');
  return Object.hasOwn(keys, code) ? t('fetchError.' + keys[code]) : fallback || t('fetchError.unknown');
}

function folderHead(folder, unreadSum, collapsed, existing) {
  const key = `h:${folder.id}`;
  let li = existing.get(key);
  if (!li) {
    li = document.createElement('li');
    li.dataset.key = key;
    li.dataset.folderId = String(folder.id);
    li.className = 'folder-head';
    li.innerHTML = '<button type="button" class="folder-arrow"></button><button type="button" class="name"></button><span class="count"></span>';
  }
  const active = state.view.kind === 'folder' && state.view.folderId === folder.id;
  li.classList.toggle('active', active);
  const arrow = li.querySelector('.folder-arrow');
  const expanded = String(!collapsed);
  if (arrow.getAttribute('aria-expanded') !== expanded) arrow.setAttribute('aria-expanded', expanded);
  const label = t(collapsed ? 'folder.expand' : 'folder.collapse');
  if (arrow.getAttribute('aria-label') !== label) arrow.setAttribute('aria-label', label);
  setText(arrow, collapsed ? '▸' : '▾');
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
  reconcileTags(collectRows('tags'));
  paintTagsSection();
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
function feedOrderTarget(sourceId, targetId) {
  const source = state.feeds.find(f => f.id === sourceId);
  const target = state.feeds.find(f => f.id === targetId);
  return !!(source && target && source.id !== target.id && source.folder_id === target.folder_id);
}

async function moveFeed(sourceId, targetId, before) {
  if (!feedOrderTarget(sourceId, targetId)) return;
  try {
    await invoke('move_feed', { feedId: sourceId, targetId, before });
    await refreshCounts();
    setStatus(t('status.feedOrderSaved'));
  } catch (error) { setStatus(t('status.settingFailed', { error: error.message }), true); }
}

function initSidebarEvents() {
  const feeds = el('feeds');
  let dragged = null;
  const clearDrop = () => feeds.querySelectorAll('.drop-before,.drop-after').forEach(row => row.classList.remove('drop-before', 'drop-after'));
  const dropTarget = ev => {
    const row = ev.target.closest('li[data-feed-id]');
    return row && feedOrderTarget(dragged, Number(row.dataset.feedId)) ? row : null;
  };
  feeds.addEventListener('dragstart', ev => {
    const row = ev.target.closest('li[data-feed-id]');
    if (!row) return;
    dragged = Number(row.dataset.feedId);
    ev.dataTransfer.effectAllowed = 'move';
    ev.dataTransfer.setData('text/plain', String(dragged));
  });
  feeds.addEventListener('dragover', ev => {
    clearDrop();
    const row = dropTarget(ev);
    if (!row) return;
    ev.preventDefault();
    ev.dataTransfer.dropEffect = 'move';
    const rect = row.getBoundingClientRect();
    row.classList.add(ev.clientY < rect.top + rect.height / 2 ? 'drop-before' : 'drop-after');
  });
  feeds.addEventListener('drop', ev => {
    const row = dropTarget(ev);
    clearDrop();
    if (row) {
      ev.preventDefault();
      const rect = row.getBoundingClientRect();
      moveFeed(dragged, Number(row.dataset.feedId), ev.clientY < rect.top + rect.height / 2);
    }
    dragged = null;
  });
  feeds.addEventListener('dragend', () => { dragged = null; clearDrop(); });
  feeds.addEventListener('click', (ev) => {
    const li = ev.target.closest('li');
    if (!li) return;
    if (li.classList.contains('folder-head')) {
      const folderId = Number(li.dataset.folderId);
      if (ev.target.closest('.folder-arrow')) toggleFolderCollapse(folderId);
      else setView({ kind: 'folder', folderId }).catch((e) => setStatus(e.message, true));
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
  if (state.view.kind === 'folder') {
    return (state.folders || []).find((f) => f.id === state.view.folderId)?.name || t('folder.viewTitle');
  }
  if (state.view.kind === 'search') return t('list.searchTitle', { q: state.query });
  if (state.view.kind === 'feed') {
    const feed = state.feeds.find((f) => f.id === state.feedId);
    return feed ? feed.title : t('list.feedFallback');
  }
  if (state.view.kind === 'tag') {
    return t('tags.viewTitle', { name: state.view.tagName || tagLabel(state.view.tagId) });
  }
  const view = VIEWS.find((v) => v.kind === state.view.kind);
  return view ? t(view.key) : t('list.all');
}

/// 单行构造：首屏与续页共用（续页只 append 新行，已有的行一个都不重建）。
function buildEntryRow(e) {
  const li = document.createElement('li');
  li.className = `${e.id === state.selectedId ? 'active' : ''} ${e.read ? 'read' : ''}`;
  li.dataset.id = String(e.id);
  const star = e.starred ? '<span class="star">★</span>' : '';
  const laterMark = `<span class="later-mark ${e.read_later ? 'on' : ''}" data-later-id="${e.id}" title="${t('list.later')}">⚑</span>`;
  // 行内 chips ≤2 + `+N`；行 hover 才显标签按钮（CSS 控制透明度，不占 hover 前的眼睛）
  const tagChips = `<span class="tag-chips">${rowTagChipsHtml(e.tags || [])}</span>`;
  const tagBtn = `<button class="row-tag-btn" data-entry-id="${e.id}" title="${t('tags.addTitle')}">#</button>`;
  const thumbnail = window.RustRssComponents.thumbnailImage(e.thumbnail_url, escapeHtml);
  li.innerHTML = window.RustRssComponents.entryContent({
    title: escapeHtml(e.title),
    meta: `<span>${escapeHtml(e.feed_title)}</span><span>${fmtTime(e.published_at)}</span>${star}${laterMark}${tagChips}${tagBtn}`,
    summary: e.summary ? escapeHtml(e.summary) : '',
    thumbnail,
  });
  li.onclick = () => openEntry(e.id, { markRead: true });
  const mark = li.querySelector('.later-mark');
  if (mark) {
    mark.onclick = (ev) => {
      ev.stopPropagation();
      // 只切这一行的稍后读标记：不动选中项、不换阅读区正文——用户可能正在读另一篇
      // （审计 P2-6：先前这里先改选中项再全量重渲正文，顺手给 B 打标记会把正在读的 A
      // 换成 B）。toggleReadLater 按传入的 id 定位行，缺省才用选中项。
      toggleReadLater(e.id).catch((err) => setStatus(err.message, true));
    };
  }
  return li;
}

/// 「已加载」口径：已加载且仍匹配当前视图**有效筛选**的行数。未读视图（或开着
/// 「隐藏已读」）里刚读过的灰显行不算——与「共 N 未读」同一口径，于是 M ≤ N 恒成立，
/// 两个数字不会自相矛盾。
function loadedCount() {
  return unreadFilteredView()
    ? state.entries.filter((e) => !e.read).length
    : state.entries.length;
}

/// 当前视图的有效筛选是否「只看未读」。store 查询层对星标 / 稍后读视图豁免
/// 「隐藏已读」、搜索不豁免——这里与那条口径一致，不另立一套。
function unreadFilteredView() {
  const kind = state.view.kind;
  if (kind === 'search') return false; // 搜索不显示总数，单独处理
  return kind === 'unread' || (listHideRead() && kind !== 'starred' && kind !== 'later');
}

/// 当前 scope（侧栏选中项）的未读数：直接取侧栏已加载状态，零额外查询。
function scopeUnread() {
  const kind = state.view.kind;
  if (kind === 'folder') {
    return (state.feeds || []).filter((f) => f.folder_id === state.view.folderId)
      .reduce((n, f) => n + f.unread, 0);
  }
  if (kind === 'feed') {
    const row = (state.feeds || []).find((f) => f.id === state.feedId);
    return row ? row.unread : null;
  }
  if (kind === 'tag') {
    const row = (state.sidebarTags || []).find((t) => t.id === state.view.tagId);
    return row ? row.unread : null;
  }
  return state.db ? state.db.unread : null;
}

/// 「全部视图 + 单源 / 单标签」的总数缓存（同一视图 + 同一有效筛选只查一次）。
let viewTotalCache = { key: null, n: null };

function viewTotalKey() {
  const kind = state.view.kind;
  const id = kind === 'feed' ? state.feedId : kind === 'tag' ? state.view.tagId : kind === 'folder' ? state.view.folderId : null;
  return `${kind}:${id ?? ''}:${unreadFilteredView() ? 'u' : 'a'}`;
}

/// 视图总数 N：同步可得就给数字，否则 null（待查库 / 本视图不适用）。
/// 侧栏的 counts 与各维度未读都是已加载状态（refreshCounts 保持新鲜）；
/// 只有「全部视图 + 单源 / 单标签」需要一次 list_scope_total。
function viewTotalSync() {
  const kind = state.view.kind;
  if (kind === 'search') return null; // FTS 总数本批不查：只显示已加载
  if (unreadFilteredView()) return scopeUnread();
  switch (kind) {
    case 'all':
      return state.db ? state.db.entries : null;
    case 'starred':
      return state.db ? state.db.starred : null;
    case 'later':
      return state.db ? state.db.later : null;
    default:
      // feed / tag：只有缓存 key 与当前（视图 + 有效筛选）一致时才用它——
      // 否则会拿到别的视图的陈旧总数（例如刚切到「全部」却沿用上一个源的 N）
      return viewTotalCache.key === viewTotalKey() ? viewTotalCache.n : null;
  }
}

/// 需要查库的视图 → 查一次总数并刷新文案；失败降级为「已加载 M 篇」+ 日志（不弹错）。
async function fetchViewTotal() {
  const kind = state.view.kind;
  if (kind !== 'feed' && kind !== 'tag' && kind !== 'folder') return;
  if (unreadFilteredView()) return; // 未读口径来自侧栏状态，不必查
  const id = kind === 'feed' ? state.feedId : kind === 'folder' ? state.view.folderId : state.view.tagId;
  if (id == null) return;
  const key = viewTotalKey();
  if (viewTotalCache.key === key) return; // 同一视图已查过（含失败：不反复重试）
  const request = { key, n: null };
  viewTotalCache = request;
  try {
    const n = await invoke('list_scope_total', { kind, id });
    if (viewTotalCache !== request || viewTotalKey() !== key) return;
    request.n = n;
  } catch (e) {
    log(`view total failed ${kind}#${id}: ${e.message}`);
  }
  if (viewTotalCache !== request || viewTotalKey() !== key) return;
  renderListCount();
  refreshSentinelFooter();
  // 机器可核对：视图总数与最终头部文案各一行（截图之外的第二条证据）
  log(`view total ${kind}#${id} n=${viewTotalCache.n ?? '-'} header=${el('list-count').textContent}`);
}

/// 条目集合变了（刷新、新条目入队）后让总数缓存失效，下次渲染重查。
function invalidateViewTotal() {
  viewTotalCache = { key: null, n: null };
}

/// 常驻「只看未读」开关的状态：与排序菜单里的「隐藏已读」同源（同一个
/// `list.hide_read` 设置、同一个 `set_list_hide_read` 命令）——两处只是同一动作的
/// 两个入口，不新增设置键、不各存一份副本。同值短路：未变则零写入。
function renderUnreadOnlyButton() {
  const btn = el('btn-unread-only');
  if (!btn) return;
  const pressed = listHideRead() ? 'true' : 'false';
  if (btn.getAttribute('aria-pressed') !== pressed) btn.setAttribute('aria-pressed', pressed);
}

/// 切换「只看未读」。按钮点击与快捷键 `U` 共用这一个动作（含 reset 语义：
/// applyListSetting 会重新查库并重建列表，分页状态一并重置）。
function toggleUnreadOnly() {
  return applyListSetting(() => invoke('set_list_hide_read', { enabled: !listHideRead() }));
}

/// 列表头计数：「已加载 M / 共 N」；有效筛选是未读时写「共 N 未读」；
/// N 未知（搜索视图或查询降级）时只写「已加载 M 篇」——宁可少写，不虚报。
function renderListCount() {
  const target = el('list-count');
  if (!state.entries.length) {
    setText(target, '');
    return;
  }
  const m = loadedCount();
  const n = viewTotalSync();
  const text =
    n == null
      ? t('list.loadedOnly', { m })
      : unreadFilteredView()
        ? t('list.loadedOfTotalUnread', { m, n })
        : t('list.loadedOfTotal', { m, n });
  if (target.textContent !== text) target.textContent = text; // 同值零写入
}

function listEmptyKey() {
  if (state.view.kind === 'search') return 'list.emptySearch';
  if (!state.feeds.length) return 'list.emptySubscriptions';
  if (state.view.kind === 'unread') return 'list.emptyUnread';
  if (state.view.kind === 'tag') return 'tags.empty';
  return 'list.empty';
}

function renderList() {
  const __t0 = performance.now();
  el('list-title').textContent = viewTitle();
  renderListCount();
  renderUnreadOnlyButton();
  void fetchViewTotal();

  const list = el('entries');
  list.innerHTML = '';
  if (!state.entries.length) {
    const li = document.createElement('li');
    li.className = 'dim';
    li.style.cursor = 'default';
    li.textContent = t(listEmptyKey());
    list.appendChild(li);
    installSentinel();
    return;
  }

  for (const e of state.entries) list.appendChild(buildEntryRow(e));
  installSentinel();
  window.__LIST_MS = +(performance.now() - __t0).toFixed(1);
  log(
    `renderList rows=${state.entries.length} withTags=${state.entries.filter((e) => (e.tags || []).length).length} ${window.__LIST_MS}ms`
  );
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

/** 列表里某 id 的行元素（行级 patch 用）；行不在当前列表里时返回 null。 */
function rowEl(id) {
  for (const li of el('entries').children) {
    if (li.dataset.id === String(id)) return li;
  }
  return null;
}

/**
 * 待翻转的条目对象：优先列表行对象（列表计数与后续整表重建都读它），行已因当前视图的
 * 过滤离开列表时（星标视图里取消星标、稍后读视图里取消标记）退回阅读区仍在展示的那个
 * 对象——正文还在屏幕上，动作按钮不能变成哑巴（否则再按一次 s/l 撤不回来）。
 */
function toggleTarget(id = state.selectedId) {
  if (id == null) return null;
  const row = state.entries.find((e) => e.id === id);
  if (row) return row;
  const shown = state.readerEntry;
  return shown && shown.id === id ? shown : null;
}

/** 把标志位写进**所有**持有该条目的对象：同一 id 可能有两个（list_entries 的行对象 /
 *  get_entry 的详情对象），只写一处会让另一处留着旧值。 */
function setEntryFlag(id, key, value) {
  for (const e of state.entries) {
    if (e.id === id) e[key] = value;
  }
  if (state.readerEntry && state.readerEntry.id === id) state.readerEntry[key] = value;
}

/**
 * 标记类动作（u/s/l）后的同一行诊断：正文区走局部 patch 时，**scrollTop 与 AI 面板状态**
 * 就是「正文 DOM 没被重渲染」的直接证据（审计 P1-2 的验收点）——Xvfb 下无人值守核对可
 * 只 grep 这一行，不必只靠肉眼比图；面板正文长度用来区分「面板还开着且内容没丢」与
 * 「被重置成 hidden 空体」。
 */
function logReaderState(action, id) {
  const panel = el('ai-panel');
  const panelState = !panel
    ? 'none'
    : panel.classList.contains('hidden')
      ? 'hidden'
      : `open:${el('ai-panel-body').textContent.length}`;
  log(`${action} id=${id} readerScrollTop=${el('reader').scrollTop} aiPanel=${panelState}`);
}

// ------------------------------------------- 列表续页（尾部哨兵 + IntersectionObserver）

/// 尾部哨兵：滚到接近底部（rootMargin 600px）就自动续下一页。
/// 每次 renderList/续页之后都换一个新哨兵节点再 observe——IntersectionObserver 只在
/// 「相交状态变化」时回调，哨兵一直留在可视区内（这批行不够填满一屏、或列表被读空
/// 变短）时不会再有通知、续页就卡住了；重新 observe 必然先给一次初始通知，正好把
/// 「还没填满就接着拉」接上。
let sentinel = null;
let listObserver = null;

/// 列表尾部行（哨兵）：每种状态都有明确表态，不再出现「哨兵突然消失」的静默尾。
///
/// - 空闲且有下一页：可点按钮「加载更多（已加载 M / 共 N）」，同时被
///   IntersectionObserver 观察——滚到底自动续批的行为与加按钮前一致；
/// - 请求在飞：按钮变「加载中…」且禁用（防抖 / 防重复 append 的既有保护不变）；
/// - 续页失败：按钮变「加载失败，点此重试」；手动点击才清 `paging.error` 重试，
///   自动触发在失败态仍被拦（不给后端制造重试风暴）；
/// - 已到末尾：不再留哨兵，改成一行明确的「已到末尾（共 N 篇）」终止态。
function installSentinel() {
  const list = el('entries');
  if (listObserver && sentinel) listObserver.unobserve(sentinel);
  if (sentinel) sentinel.remove();
  sentinel = null;
  // 空列表交给空态占位，不画尾部行
  if (!state.entries.length) return;

  sentinel = document.createElement('li');
  sentinel.className = 'load-sentinel dim';
  if (paging.exhausted || state.view.kind === 'search') {
    // 终止态：明确写出来，避免「没有尾部行 = 还有更多」的歧义
    sentinel.textContent = sentinelTerminalText();
    list.appendChild(sentinel);
    return;
  }
  if (!sentinelPending()) return;

  const btn = document.createElement('button');
  btn.type = 'button';
  btn.textContent = sentinelLabel();
  btn.onclick = () => loadMore({ manual: true });
  sentinel.appendChild(btn);
  refreshSentinelFooter();
  list.appendChild(sentinel);
  if (!paging.error) {
    if (!listObserver) {
      listObserver = new IntersectionObserver(onSentinel, { root: list, rootMargin: '600px' });
    }
    listObserver.observe(sentinel);
  }
}

/// 是否还有下一页可拉（失败态也算——手动重试要用）
function sentinelPending() {
  return !paging.exhausted && (paging.cursor != null || paging.error);
}

/// 尾部按钮文案：失败态 → 重试；正常 → 进度（总数未知时退化为只报已加载）
function sentinelLabel() {
  if (paging.error) return t('list.loadMoreRetry');
  const m = loadedCount();
  const n = viewTotalSync();
  return n == null ? t('list.loadMoreNoTotal', { m }) : t('list.loadMore', { m, n });
}

/// 终止行文案：已到末尾（共 N 篇）。搜索本版一次性 200 条、不分页——
/// 如实写「已加载 M 篇」，不声称 FTS 总数。
function sentinelTerminalText() {
  return state.view.kind === 'search'
    ? t('list.loadedOnly', { m: loadedCount() })
    : t('list.allLoaded', { n: viewTotalSync() ?? loadedCount() });
}

/// 尾部行按最新状态重算：按钮态（含在飞禁用）与终止行文本都从同一批函数取值。
/// 计数刷新（refreshCounts）与续页开始时都调它——否则会话内标读后尾部会停在旧数字上，
/// 尤其终止行没有按钮、以前根本没人重算它（聚合复核 B1 的同类残留）。
function refreshSentinelFooter() {
  if (!sentinel) return;
  const btn = sentinel.querySelector('button');
  if (btn) {
    if (btn.disabled !== paging.loading) btn.disabled = paging.loading;
    setText(btn, paging.loading ? t('list.loadingMore') : sentinelLabel());
    return;
  }
  setText(sentinel, sentinelTerminalText());
}

function onSentinel(records) {
  if (records.some((r) => r.isIntersecting)) loadMore();
}

function renderReaderEmpty() {
  state.readerFeedId = null;
  state.readerEntry = null;
  // 空状态把三套语义一次说清：星标=收藏 · 稍后读=待读 · 标签=主题分类（i18n 双语）
  el('reader').innerHTML = `<div class="reader-empty">
      <p>${t('reader.empty')}</p>
      <p class="dim">${t('reader.shortcuts')}</p>
      <p class="dim">${t('tags.semantics')}</p>
    </div>`;
}

function renderReader(entry) {
  state.readerFeedId = entry.feed_id;
  state.readerEntry = entry;
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

  reader.innerHTML = window.RustRssComponents.readerHead({
    title: escapeHtml(entry.title),
    meta: `<span>${escapeHtml(entry.feed_title)}</span><span>${fmtTime(entry.published_at)}</span>
      ${entry.author ? `<span>${escapeHtml(entry.author)}</span>` : ''}
      <span class="tag-bar" id="reader-tags">${readerTagChipsHtml(entry)}</span>`,
  });
  reader.insertAdjacentHTML('beforeend', `
    <div class="reader-actions">
      <button id="act-aa" aria-haspopup="dialog" title="${t('theme.reading')}">Aa</button>
      <button id="act-read">${entry.read ? t('reader.markUnread') : t('reader.markRead')}</button>
      <button id="act-star">${entry.starred ? t('reader.removeStar') : t('reader.addStar')}</button>
      <button id="act-later" class="${entry.read_later ? 'later-active' : ''}">${entry.read_later ? t('reader.removeLater') : t('reader.markLater')}</button>
      ${entry.url ? `<button id="act-open">${t('reader.openInBrowser')}</button><button id="act-copy">${t('reader.copyLink')}</button>` : ''}
      ${entry.needs_fulltext ? `<button id="act-fulltext" title="${t('reader.fetchFulltextTitle')}">${t('reader.fetchFulltext')}</button>` : ''}
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
    ${window.RustRssComponents.article(body)}`);

  mark('innerHTML-set');
  // 高亮必须在正文插入 DOM 之后跑（hljs 需要真实节点）；
  // 输入是 sanitize 产物，hljs 输出不回灌 sanitize 流程。
  normalizeDiffBlocks(reader.querySelector('.article'));
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
  // chips 的自证行：数量/名称/第一个 chip 的屏幕矩形都在里面——无人值守时
  // 「chips 渲染出来了」与「点哪里」两件事都从这一行读，不必靠肉眼比图。
  log(
    `readerTags id=${entry.id} n=${(entry.tags || []).length} chips=${(entry.tags || []).map((x) => x.name).join('|') || '-'} rect=${tagChipRect('#reader-tags .tag-chip')}`
  );

  el('act-aa').onclick = openAa;
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
  // 只有摘要型条目的行会带这个按钮（needs_fulltext 由 Rust 侧判定，列表行恒为 false）
  if (entry.needs_fulltext) el('act-fulltext').onclick = () => fetchFulltext(entry.id);

  // 正文里的链接交给系统浏览器，避免在应用内导航走丢
  reader.querySelectorAll('a[href]').forEach((a) => {
    a.onclick = (ev) => {
      ev.preventDefault();
      invoke('open_external', { url: a.getAttribute('href') }).catch((e) => setStatus(e.message, true));
    };
  });
  reader.scrollTop = 0;
}

/// 「获取全文」在飞标记：同一时刻只允许一个请求（按钮 disabled 只活到下一次重渲染，
/// 而重渲染随时可能发生——视图切换、后台刷新完成、AI 面板操作；靠 DOM 记状态会漏防）。
let fulltextInFlight = null;

/**
 * 抓原文页 → 提取正文 → 写回库，成功后用返回的 EntryRow 重渲染阅读区。
 *
 * 三步都与后端 `fetch_fulltext` 的分工一致：幂等、体积闸门、提取都在 Rust 侧，
 * 前端只负责「显示入口 → 转 loading → 用回读的行替换正文 / 报错」。
 * 失败时**一个正文 DOM 都不动**：界面继续显示原摘要，按钮恢复可点，用户可重试。
 */
async function fetchFulltext(entryId) {
  if (fulltextInFlight !== null) return;
  fulltextInFlight = entryId;
  const btn = el('act-fulltext');
  if (btn) {
    btn.disabled = true;
    btn.textContent = t('reader.fetching');
  }
  setStatus(t('reader.fetching'));
  try {
    const row = await invoke('fetch_fulltext', { entryId });
    // 在飞期间用户可能翻到了别的文章：那就只更新数据，不抢当前阅读焦点
    if (state.selectedId === entryId) renderReader(row);
    setStatus(t('status.fulltextDone'));
    log(
      `fetch_fulltext ok entry=${entryId} html=${row.content_html ? row.content_html.length : 0}chars stillNeeds=${row.needs_fulltext}`
    );
  } catch (e) {
    setStatus(t('status.fulltextFailed', { error: e.message }), true);
    log(`fetch_fulltext failed entry=${entryId}: ${e.message}`);
    if (btn) {
      btn.disabled = false;
      btn.textContent = t('reader.fetchFulltext');
    }
  } finally {
    fulltextInFlight = null;
  }
}

// ---------------------------------------------------------------- 标签（chips / 选择器 / 标签视图）
//
// 数据口径全在 core：chips 的标签来自 `EntryRow.tags`；选择器顺序来自
// `list_tags(recent_first=true)`（`last_used_at DESC`，无记录回退 sort_order/名称）；
// 每个标签的未读数来自 `TagRow.unread`。前端只做渲染、键盘与「按 id 局部 patch」——
// 不自己排序、不自己数数（两套口径必然漂移）。

/** 列表行最多显示的 chips 数（超出显示 `+N`） */
const ROW_TAG_CHIPS = 2;

/** 标签色值只接受 core 校验过的 `#rrggbb`；其余（含 null）一概退回默认色，不进 style */
function tagColorStyle(tag) {
  return /^#[0-9a-f]{6}$/i.test(tag.color || '') ? ` style="--tag-color:${tag.color}"` : '';
}

/** 单个 chip（阅读器与列表行共用同一份构造：转义与 data 属性只有一处） */
function tagChipHtml(tag) {
  return `<span class="tag-chip" data-tag-id="${tag.id}" title="${escapeHtml(tag.name)}"${tagColorStyle(tag)}>${escapeHtml(tag.name)}</span>`;
}

/** 列表行 chips：≤2 个 + `+N`（被折叠的标签名进 title，悬停仍能看全） */
function rowTagChipsHtml(tags) {
  const shown = tags.slice(0, ROW_TAG_CHIPS);
  let html = shown.map(tagChipHtml).join('');
  if (tags.length > shown.length) {
    const all = tags.map((x) => x.name).join(' / ');
    html += `<span class="tag-chip more" title="${escapeHtml(t('tags.chipMore', { n: tags.length - shown.length }))}：${escapeHtml(all)}">+${tags.length - shown.length}</span>`;
  }
  return html;
}

/** 阅读器 meta 行的 chips + 「＋标签」按钮（整体重写这个容器就是局部 patch） */
function readerTagChipsHtml(entry) {
  const chips = (entry.tags || []).map(tagChipHtml).join('');
  return `<span class="tag-chips">${chips}</span><button class="tag-add" data-entry-id="${entry.id}" title="${t('tags.addTitle')}">${t('tags.add')}</button>`;
}

/** 第一个 chip 的屏幕矩形（无人值守点击定位与「chips 可见」的证据行用） */
function tagChipRect(selector) {
  const node = document.querySelector(selector);
  if (!node) return '-';
  const r = node.getBoundingClientRect();
  return `${Math.round(r.left)},${Math.round(r.top)},${Math.round(r.width)},${Math.round(r.height)}`;
}

function tagById(id) {
  return (state.tags || []).find((tg) => tg.id === id) || null;
}

/** 标签名（缓存未命中时退回 chip 文本/`#id`：渲染不依赖缓存先到位） */
function tagLabel(id, fallback) {
  const tag = tagById(id);
  return tag ? tag.name : fallback || `#${id}`;
}

/** 刷新标签缓存。`recentFirst` 缺省 = 选择器口径（最近使用优先）；侧栏口径传 false */
async function refreshTagCache({ recentFirst = true } = {}) {
  state.tags = await invoke('list_tags', { recentFirst });
  return state.tags;
}

/** 条目身上当前的标签 id 集合（阅读区详情对象 / 列表行对象，两处都可能持有） */
function entryTags(id) {
  const entry =
    state.readerEntry && state.readerEntry.id === id
      ? state.readerEntry
      : state.entries.find((e) => e.id === id);
  return (entry && entry.tags) || [];
}

/** 把回读到的标签写进**所有**持有该条目的对象（同 setEntryFlag 的两对象口径） */
function setEntryTags(id, tags) {
  for (const e of state.entries) {
    if (e.id === id) e.tags = tags;
  }
  if (state.readerEntry && state.readerEntry.id === id) state.readerEntry.tags = tags;
}

/** 列表行的 chips 局部 patch：只重写该行的 chips 容器，不重建行也不重建列表 */
function patchRowTags(id) {
  const li = rowEl(id);
  const e = state.entries.find((row) => row.id === id);
  if (!li || !e) return;
  const box = li.querySelector('.tag-chips');
  if (box) box.innerHTML = rowTagChipsHtml(e.tags || []);
}

/** 阅读器 chips 局部 patch：只重写 chips 容器（正文 DOM / 滚动位置 / AI 面板零变化） */
function patchReaderTags() {
  const box = el('reader-tags');
  if (!box || !state.readerEntry) return;
  box.innerHTML = readerTagChipsHtml(state.readerEntry);
}

/**
 * 打标/取消的收口：回读该条目（`EntryRow.tags` 的唯一来源）→ 写回两个对象 →
 * 局部 patch 两处 DOM → 刷新标签缓存（计数/最近使用顺序）→ 刷新侧栏聚合计数 →
 * 标签视图里取消当前筛选标签时把该行移出列表。
 */
async function afterTagChange(id, tag, attached) {
  const row = await invoke('get_entry', { id });
  if (row) setEntryTags(id, row.tags || []);
  patchRowTags(id);
  patchReaderTags();
  await refreshTagCache();
  refreshCountsSoon();
  if (state.view.kind === 'tag') {
    invalidateViewTotal();
    if (!attached && state.view.tagId === tag.id) dropRowFromList(id);
    renderListCount();
    refreshSentinelFooter();
    void fetchViewTotal();
  }
  setStatus(t(attached ? 'tags.assigned' : 'tags.unassigned', { name: tag.name }));
  log(
    `tags:${attached ? 'assign' : 'unassign'} entry=${id} tag=${tag.id} name=${tag.name} ` +
      `chips=${entryTags(id).map((x) => x.name).join('|') || '-'} rect=${tagChipRect('#reader-tags .tag-chip')} listRows=${state.entries.length}`
  );
}

/** 给条目附加/取消一个标签（选择器行与 Enter 确认都走这里，幂等由 core 保证） */
async function toggleTagOnEntry(entryId, tag) {
  const attached = entryTags(entryId).some((x) => x.id === tag.id);
  try {
    if (attached) await invoke('unassign_tags', { entryId, tagIds: [tag.id] });
    else await invoke('assign_tags', { entryId, tagIds: [tag.id] });
  } catch (e) {
    setStatus(e.message, true);
    log(`tags:${attached ? 'unassign' : 'assign'} failed entry=${entryId} tag=${tag.id}: ${e.message}`);
    return;
  }
  await afterTagChange(entryId, tag, !attached);
}

/** Enter 新建并附加。重名（core 判定，大小写不敏感）走既有报错提示路径，不静默吞 */
async function createAndAttachTag(entryId, name) {
  try {
    const created = await invoke('create_tag', { name });
    await invoke('assign_tags', { entryId, tagIds: [created.id] });
    await afterTagChange(entryId, created, true);
    log(`tags:create name=${created.name} id=${created.id} entry=${entryId}`);
    return created;
  } catch (e) {
    setStatus(e.message, true);
    log(`tags:create failed name=${name}: ${e.message}`);
    return null;
  }
}

// ---- 选择器：state.tags（最近使用优先序）+ type-ahead 过滤 + ↑↓/Enter/Esc

let tagPickerEntryId = null;
let tagPickerIndex = 0;
/** 当前候选行（含「新建并附加」合成行）；渲染与键盘选择读同一份，不会各走各的 */
let tagPickerRows = [];

function tagPickerOpen() {
  return !el('tag-picker-overlay').classList.contains('hidden');
}

async function openTagPicker(entryId) {
  if (entryId == null) return;
  tagPickerEntryId = entryId;
  // 每次打开重取一次：两次打标（last_used_at 被推进）后最近使用的那条必然排前
  await refreshTagCache();
  const entry =
    state.readerEntry && state.readerEntry.id === entryId
      ? state.readerEntry
      : state.entries.find((e) => e.id === entryId);
  el('tag-picker-title').textContent = entry
    ? `${t('tags.pickerTitle')} · ${entry.title}`
    : t('tags.pickerTitle');
  const input = el('tag-picker-input');
  input.value = '';
  tagPickerIndex = 0;
  renderTagPicker('');
  el('tag-picker-overlay').classList.remove('hidden');
  input.focus();
  log(
    `tagPicker:open entry=${entryId} attached=${entryTags(entryId).map((x) => x.name).join('|') || '-'} order=${(state.tags || []).map((x) => x.name).join('|') || '-'}`
  );
}

function closeTagPicker(reason) {
  if (!tagPickerOpen()) return;
  el('tag-picker-overlay').classList.add('hidden');
  el('tag-picker-input').value = '';
  tagPickerRows = [];
  tagPickerIndex = 0;
  const id = tagPickerEntryId;
  tagPickerEntryId = null;
  // 关闭（含 Esc/点遮罩）本身不改任何数据、不动视图/搜索/设置——只有这一行日志
  log(`tagPicker:close reason=${reason} entry=${id} view=${state.view.kind}`);
}

/** 按输入重渲染候选行；`q` 为空 = 全部标签（core 的最近使用优先序原样） */
function renderTagPicker(q) {
  const query = (q || '').trim();
  const lower = query.toLowerCase();
  const matched = (state.tags || []).filter((tg) => tg.name.toLowerCase().includes(lower));
  // 大小写不敏感的全名命中优先：这时 Enter 是「附加已有」，不去撞唯一约束报错
  const exact = matched.some((tg) => tg.name.toLowerCase() === lower);
  const createName = query && !exact ? query : null;
  tagPickerRows = [];
  // 候选在前、新建在后：输入 `gam` 而库里已有 `gamma` 时，默认选中是附加 gamma
  // （实机跑出来的：新建排第一时 Enter 会凭空造一个 `gam`，近义新标签是脏数据）。
  // 真要新建一个与现有标签前缀相同的新标签，↓ 到末行回车即可（新建行高亮色区分）。
  for (const tg of matched) tagPickerRows.push({ kind: 'tag', tag: tg });
  if (createName) tagPickerRows.push({ kind: 'create', name: createName });
  if (tagPickerIndex >= tagPickerRows.length) tagPickerIndex = 0;

  const attachedIds = new Set(entryTags(tagPickerEntryId).map((x) => x.id));
  const list = el('tag-picker-list');
  list.innerHTML = tagPickerRows.length
    ? tagPickerRows
        .map((row, i) => {
          const active = i === tagPickerIndex ? ' active' : '';
          if (row.kind === 'create') {
            return `<li data-index="${i}" class="create${active}">${escapeHtml(t('tags.pickerCreate', { name: row.name }))}</li>`;
          }
          const tg = row.tag;
          const on = attachedIds.has(tg.id);
          return (
            `<li data-index="${i}" data-tag-id="${tg.id}" class="${active.trim()}${on ? ' attached' : ''}"${tagColorStyle(tg)} title="${on ? t('tags.pickerAttached') : escapeHtml(tg.name)}">` +
            `<span class="tag-dot"></span><span class="tag-name">${escapeHtml(tg.name)}</span>` +
            `<span class="tag-unread dim">${t('tags.pickerUnread', { n: tg.unread })}</span>` +
            `<span class="tag-mark">${on ? '✓' : ''}</span></li>`
          );
        })
        .join('')
    : `<li class="dim empty">${t('tags.pickerEmpty')}</li>`;
  const activeRow = list.querySelector('li.active');
  if (activeRow) activeRow.scrollIntoView({ block: 'nearest' });
  log(
    `tagPicker:filter q=${query || '-'} rows=${tagPickerRows.length} sel=${tagPickerRows[tagPickerIndex] ? tagPickerRows[tagPickerIndex].kind === 'create' ? 'create:' + tagPickerRows[tagPickerIndex].name : tagPickerRows[tagPickerIndex].tag.name : '-'} order=${(state.tags || []).map((x) => x.name).join('|') || '-'}`
  );
}

/** Enter / 点击一行：合成行走新建，标签行走附加/取消 */
async function confirmTagPickerRow() {
  const row = tagPickerRows[tagPickerIndex];
  if (!row) return;
  if (row.kind === 'create') {
    await createAndAttachTag(tagPickerEntryId, row.name);
    const input = el('tag-picker-input');
    input.value = '';
    tagPickerIndex = 0;
    renderTagPicker('');
    input.focus();
    return;
  }
  await toggleTagOnEntry(tagPickerEntryId, row.tag);
  renderTagPicker(el('tag-picker-input').value);
}

/**
 * 选择器事件：输入过滤 + 键盘。键盘用**捕获阶段**监听，Esc/Enter/↑↓ 都在这里
 * 消化掉（stopPropagation），因此全局快捷键（j/k/u/s/l/t…）在选择器开着时一个
 * 都不会被触发——Esc 的「无副作用」靠的是这两层，而不是靠全局处理器自觉。
 * 其它字符键不拦：焦点在输入框，type-ahead 要能打进去。
 */
function initTagPickerEvents() {
  const input = el('tag-picker-input');
  input.addEventListener('input', () => {
    tagPickerIndex = 0;
    renderTagPicker(input.value);
  });
  document.addEventListener(
    'keydown',
    (ev) => {
      if (!tagPickerOpen()) return;
      if (ev.key === 'ArrowDown' || ev.key === 'ArrowUp') {
        ev.preventDefault();
        ev.stopPropagation();
        if (!tagPickerRows.length) return;
        const step = ev.key === 'ArrowDown' ? 1 : -1;
        tagPickerIndex = (tagPickerIndex + step + tagPickerRows.length) % tagPickerRows.length;
        renderTagPicker(input.value);
        return;
      }
      if (ev.key === 'Enter') {
        ev.preventDefault();
        ev.stopPropagation();
        confirmTagPickerRow().catch((e) => setStatus(e.message, true));
        return;
      }
      if (ev.key === 'Escape') {
        ev.preventDefault();
        ev.stopPropagation();
        closeTagPicker('esc');
      }
    },
    true
  );
  el('tag-picker-close').onclick = () => closeTagPicker('button');
  el('tag-picker-overlay').addEventListener('click', (ev) => {
    if (ev.target === el('tag-picker-overlay')) closeTagPicker('backdrop');
  });
  el('tag-picker-list').addEventListener('click', (ev) => {
    const li = ev.target.closest('li[data-index]');
    if (!li) return;
    tagPickerIndex = Number(li.dataset.index);
    confirmTagPickerRow().catch((e) => setStatus(e.message, true));
  });
}

/**
 * chips 与「＋标签」的容器级代理（捕获阶段）：chip → 按该标签筛选列表；
 * 「＋」/行按钮 → 打开选择器。捕获阶段是必须的——列表行的 `li.onclick` 会打开文章，
 * 冒泡阶段再拦就已经晚了（点击 chip 会先打开文章再筛选）。
 */
function initTagEvents() {
  document.addEventListener(
    'click',
    (ev) => {
      const chip = ev.target.closest('.tag-chip[data-tag-id]');
      if (chip) {
        ev.preventDefault();
        ev.stopPropagation();
        const tagId = Number(chip.dataset.tagId);
        setView({ kind: 'tag', tagId, tagName: tagLabel(tagId, chip.textContent) });
        return;
      }
      const add = ev.target.closest('.tag-add, .row-tag-btn');
      if (!add) return;
      ev.preventDefault();
      ev.stopPropagation();
      const id = Number(add.dataset.entryId);
      openTagPicker(Number.isFinite(id) && id > 0 ? id : state.readerEntry?.id ?? state.selectedId).catch(
        (e) => setStatus(e.message, true)
      );
    },
    true
  );
}

/**
 * 启动自检：全局快捷键表不冲突。
 *
 * 键位不是另抄一份清单，而是从**真正生效的那个处理函数**的源码里抽 `case '<键>'`
 * ——清单自说自话是两处漂移的源头（改了 switch 忘改清单，测试还绿）。断言：
 * ① 一个键只绑定一次；② `t` 在表里；③ 既有键位一个都没丢。
 */
function selfTestShortcutKeys() {
  const keys = [...onGlobalKeydown.toString().matchAll(/case '([^']+)'/g)].map((m) => m[1]);
  const problems = [];
  const seen = new Set();
  for (const k of keys) {
    if (seen.has(k)) problems.push(`键位重复绑定: ${k}`);
    seen.add(k);
  }
  if (!seen.has('t')) problems.push('缺少 t 快捷键');
  for (const k of ['j', 'k', 'u', 's', 'l', 'r', 'g', 'G', 'U', 'A']) {
    if (!seen.has(k)) problems.push(`既有键位丢失: ${k}`);
  }
  log(
    problems.length
      ? `shortcut selftest FAILED: ${problems.join('; ')}`
      : `shortcut selftest ok (keys=${keys.join(' ')} ; 输入区/覆盖层里一律不触发)`
  );
  return problems.length === 0;
}

// ---------------------------------------------------------------- 标签管理（侧栏区 / 拖拽排序 / 右键菜单）
//
// 侧栏顺序（置顶优先 → 手动顺序 → 名称）、未读计数、颜色全部来自 core 的
// `list_tags`（`sidebar_data.tags` 一次锁带回来）——前端只渲染，不自己排序也不自己
// 数数。拖拽落库把**整份可见顺序**交给 core 的 `reorder_tags`（单事务按下标写
// `sort_order`），不做逐行 IPC；DOM 侧只挪被拖的那一个节点，不重建列表。

/** 预设色板：每个色在浅色 `--bg-panel`(#ffffff) 与深色 `#1a1d23` 上对比度都 ≥ 3:1
 *  （WCAG 非文本图形底线；启动自检 `selfTestTagPalette` 现算，深浅两主题各跑一次启动
 *  即可核对）。颜色只走圆点，不改文字色——两主题下都靠圆点自身可辨。 */
const TAG_PALETTE = [
  { key: 'tags.color.red', color: '#e5484d' },
  { key: 'tags.color.orange', color: '#e8590c' },
  { key: 'tags.color.green', color: '#30a46c' },
  { key: 'tags.color.teal', color: '#12a594' },
  { key: 'tags.color.blue', color: '#3e63dd' },
  { key: 'tags.color.violet', color: '#8e4ec6' },
  { key: 'tags.color.pink', color: '#d6409f' },
  { key: 'tags.color.slate', color: '#6b7280' },
];

/** `#rrggbb`（core 收的颜色口径：7 字符、# 开头、全十六进制） */
function validTagColor(color) {
  return /^#[0-9a-f]{6}$/i.test(color || '');
}

/** 把 core 的颜色落到节点的 CSS 变量上；无颜色则清掉变量（CSS 回退到 --fg-dim） */
function applyTagColor(node, tag) {
  if (validTagColor(tag.color)) node.style.setProperty('--tag-color', tag.color.toLowerCase());
  else node.style.removeProperty('--tag-color');
}

/** 当前颜色的中文/英文名（右键菜单父项回显用；库里是色板外的值也不算错） */
function tagColorLabel(tag) {
  const hit = TAG_PALETTE.find((c) => c.color === (tag.color || '').toLowerCase());
  return hit ? t(hit.key) : t('tags.color.none');
}

/** sRGB 相对亮度（WCAG 2.x，对比度断言用） */
function relLuminance(hex) {
  const [r, g, b] = [1, 3, 5]
    .map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function contrastRatio(a, b) {
  const la = relLuminance(a);
  const lb = relLuminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/**
 * 启动自检：色板可用性。
 * ① 每个色都是 core 收的 `#rrggbb` 格式且不重复（格式不对 core 直接报 invalid，
 *    右键菜单会变成一排报错）；
 * ② 每个色在**当前主题**的 `--bg-panel` 上对比度 ≥ 3:1——日志里的
 *    theme/bgPanel/minContrast 就是「该主题下圆点辨识得出」的机械证据（深浅主题
 *    各跑一次启动即可覆两个主题）。
 */
function selfTestTagPalette() {
  const problems = [];
  const seen = new Set();
  for (const c of TAG_PALETTE) {
    if (!/^#[0-9a-f]{6}$/.test(c.color)) problems.push(`色板色值非法: ${c.color}`);
    if (seen.has(c.color)) problems.push(`色板色值重复: ${c.color}`);
    seen.add(c.color);
  }
  const panel = getComputedStyle(document.documentElement).getPropertyValue('--bg-panel').trim();
  const min = validTagColor(panel)
    ? Math.min(...TAG_PALETTE.map((c) => contrastRatio(c.color, panel)))
    : 0;
  if (validTagColor(panel) && min < 3) {
    problems.push(`色板在 --bg-panel=${panel} 上最低对比度 ${min.toFixed(2)} < 3:1`);
  }
  const theme = document.documentElement.dataset.theme || 'system';
  log(
    problems.length
      ? `tagPalette selftest FAILED: ${problems.join('; ')}`
      : `tagPalette selftest ok (n=${TAG_PALETTE.length} theme=${theme} bgPanel=${panel} minContrast=${min.toFixed(2)} ratios=${TAG_PALETTE.map((c) => contrastRatio(c.color, validTagColor(panel) ? panel : '#000000').toFixed(2)).join('/')})`
  );
  return problems.length === 0;
}

/** 侧栏标签区的行（keyed reconcile 复用；事件全走容器代理，行复用不重挂监听） */
function tagRow(tag, existing) {
  const key = `t:${tag.id}`;
  let li = existing.get(key);
  if (!li) {
    li = document.createElement('li');
    li.dataset.key = key;
    li.dataset.tagId = String(tag.id);
    li.draggable = true; // 原生 drag 事件（拖拽排序的唯一入口）
    li.innerHTML = '<span class="tag-dot"></span><span class="name"></span><span class="count"></span>';
  }
  li.className = state.view.kind === 'tag' && state.view.tagId === tag.id ? 'active' : '';
  applyTagColor(li, tag);
  setText(li.querySelector('.name'), tag.name);
  // 未读计数照出（0 也显示）：与源行同一口径，免得「有的行有数字有的没有」
  setText(li.querySelector('.count'), String(tag.unread));
  li.title = t('sidebar.tagRowTitle', { name: tag.name });
  return li;
}

/** 标签区行渲染：顺序就是 core 给的顺序（置顶优先），前端不重排 */
function reconcileTags(existing) {
  const tags = state.sidebarTags || [];
  reconcileChildren(el('tags'), tags.map((tag) => tagRow(tag, existing)));
  setText(el('tags-meta'), tags.length ? t('sidebar.tagCount', { n: tags.length }) : '');
}

/** 折叠状态落到 DOM（箭头 + 列表/空态显隐） */
function paintTagsSection() {
  const tags = state.sidebarTags || [];
  setText(el('tags-arrow'), state.tagsCollapsed ? '▸' : '▾');
  el('tags').classList.toggle('hidden', !!state.tagsCollapsed);
  el('tags-empty').classList.toggle('hidden', !!state.tagsCollapsed || tags.length > 0);
}

/** 折叠/展开标签区：状态落库（跨会话保持，与文件夹折叠同一套设置存储） */
function toggleTagsCollapse() {
  state.tagsCollapsed = !state.tagsCollapsed;
  invoke('set_tags_collapsed', { collapsed: state.tagsCollapsed }).catch(() => {});
  renderSidebar();
  log(
    `tagsSection: collapsed=${state.tagsCollapsed ? 1 : 0} head=${rectText(el('tags-head'))} ` +
      `rows=${(state.sidebarTags || []).length} rects=${tagSidebarRects()}`
  );
}

/** 标签区的标签名序列（诊断日志用；拖拽前后/重启后对着 sqlite 的 sort_order 核） */
function tagOrderText() {
  return (state.sidebarTags || []).map((tg) => tg.name).join('|') || '-';
}

/** 元素的屏幕矩形文本（`left,top,w,h`）——诊断与无人值守点击/拖拽定位用 */
function rectText(node) {
  const r = node.getBoundingClientRect();
  return `${Math.round(r.left)},${Math.round(r.top)},${Math.round(r.width)},${Math.round(r.height)}`;
}

/** 侧栏标签区各行的屏幕矩形（`名=left,top,w,h`，行序即当前顺序） */
function tagSidebarRects() {
  return (
    [...el('tags').children]
      .map((li) => `${li.querySelector('.name').textContent}=${rectText(li)}`)
      .join('|') || '-'
  );
}

// ---- 拖拽排序：原生 drag 事件 + 整份顺序单事务落库

/** 正在拖拽的标签（`{id, pinned}`；null = 没有拖拽） */
let tagDragState = null;

/** 当前落点（行 + 插在它前/后）；无落点 = null */
let tagDropHint = null;

function clearTagDropHint() {
  for (const li of el('tags').children) li.classList.remove('drop-before', 'drop-after');
  tagDropHint = null;
}

/** 落点行 + 插到它前还是后（以行高中线为界） */
function tagDropSpot(ev) {
  const li = ev.target.closest('li[data-tag-id]');
  if (!li) return null;
  const rect = li.getBoundingClientRect();
  return { tagId: Number(li.dataset.tagId), li, before: ev.clientY < rect.top + rect.height / 2 };
}

/**
 * 把「被拖的标签」插到目标行前/后，返回整份新顺序的 id 数组。
 * 整份列表（含置顶组）一并交给 core，于是置顶组的相对顺序也不会被拖乱——core 的
 * ORDER BY 里 `pinned DESC` 在前，置顶永远在最前。
 */
function nextTagOrder(draggedId, targetId, before) {
  const ids = (state.sidebarTags || []).map((tg) => tg.id);
  const from = ids.indexOf(draggedId);
  if (from < 0 || targetId === draggedId || !ids.includes(targetId)) return null;
  ids.splice(from, 1);
  ids.splice(ids.indexOf(targetId) + (before ? 0 : 1), 0, draggedId);
  return ids;
}

/**
 * 落库新顺序（core 单事务）。DOM 侧只**移动**被拖的那一个节点：reconcileChildren
 * 按 key 归位，created/removed 两个计数（日志里）就是「不重建整个列表」的证据。
 */
async function persistTagOrder(ids) {
  const byId = new Map((state.sidebarTags || []).map((tg) => [tg.id, tg]));
  const next = ids.map((id) => byId.get(id)).filter(Boolean);
  const before = new Set(el('tags').children);
  state.sidebarTags = next;
  renderSidebar();
  const after = [...el('tags').children];
  const created = after.filter((n) => !before.has(n)).length;
  const removed = [...before].filter((n) => !after.includes(n)).length;
  log(
    `tagReorder: order=${tagOrderText()} created=${created} removed=${removed} persisted=pending rects=${tagSidebarRects()}`
  );
  try {
    await invoke('reorder_tags', { tagIds: ids });
    log(`tagReorder: order=${tagOrderText()} created=${created} removed=${removed} persisted=ok`);
    setStatus(t('tags.reordered'));
  } catch (e) {
    log(`tagReorder: order=${tagOrderText()} created=${created} removed=${removed} persisted=fail err=${e.message}`);
    setStatus(t('tags.reorderFailed', { error: e.message }), true);
    await refreshCounts(); // 回滚显示：以库里的顺序为准
  }
}

/** 标签区事件：折叠头 + 行点击筛选 + 右键管理 + 原生拖拽排序（全部容器代理） */
function initTagSectionEvents() {
  const head = el('tags-head');
  head.addEventListener('click', toggleTagsCollapse);
  // role=button 的键盘可达性（div 不会自己响应 Enter/Space）
  head.addEventListener('keydown', (ev) => {
    if (ev.key === 'Enter' || ev.key === ' ') {
      ev.preventDefault();
      toggleTagsCollapse();
    }
  });

  const list = el('tags');
  const tagFromRow = (node) =>
    (state.sidebarTags || []).find((tg) => tg.id === Number(node.dataset.tagId));

  list.addEventListener('click', (ev) => {
    const li = ev.target.closest('li[data-tag-id]');
    if (!li) return;
    const tag = tagFromRow(li);
    if (!tag) return;
    setView({ kind: 'tag', tagId: tag.id, tagName: tag.name }).catch((e) => setStatus(e.message, true));
  });

  list.addEventListener('contextmenu', (ev) => {
    const li = ev.target.closest('li[data-tag-id]');
    if (!li) return;
    ev.preventDefault();
    ev.stopPropagation();
    const tag = tagFromRow(li);
    if (tag) openTagMenu(ev, tag);
  });

  list.addEventListener('dragstart', (ev) => {
    const li = ev.target.closest('li[data-tag-id]');
    const tag = li && tagFromRow(li);
    if (!tag) return;
    tagDragState = { id: tag.id, pinned: !!tag.pinned };
    li.classList.add('dragging');
    // 原生 DnD 需要在 dataTransfer 里放点东西才会真正起拖（WebKit 同口径）
    ev.dataTransfer.effectAllowed = 'move';
    ev.dataTransfer.setData('text/plain', String(tag.id));
    log(`tagDrag:start id=${tag.id} name=${tag.name} pinned=${tag.pinned ? 1 : 0} order=${tagOrderText()}`);
  });

  list.addEventListener('dragover', (ev) => {
    if (!tagDragState) return;
    const spot = tagDropSpot(ev);
    const tag = spot && tagFromRow(spot.li);
    // 置顶组与普通组不混排：core 的 ORDER BY 里置顶恒在前，跨组落库也不会生效
    // （拖了像没反应），所以干脆不接受跨组落点，也不给插入指示。
    if (!spot || !tag || !!tag.pinned !== tagDragState.pinned) {
      clearTagDropHint();
      return;
    }
    ev.preventDefault(); // 允许 drop（不 preventDefault 掉 drop 永远不来）
    ev.dataTransfer.dropEffect = 'move';
    if (tagDropHint && tagDropHint.tagId === spot.tagId && tagDropHint.before === spot.before) return;
    clearTagDropHint();
    spot.li.classList.add(spot.before ? 'drop-before' : 'drop-after');
    tagDropHint = spot;
  });

  // 拖出列表/拖出窗口：收掉插入指示（拖回来自会重画）
  list.addEventListener('dragleave', (ev) => {
    if (!list.contains(ev.relatedTarget)) clearTagDropHint();
  });

  list.addEventListener('drop', (ev) => {
    if (!tagDragState) return;
    const spot = tagDropHint;
    const dragged = tagDragState;
    tagDragState = null;
    clearTagDropHint();
    if (!spot) return; // 跨组/没落在行上：不做事（元素留在原位）
    ev.preventDefault();
    const ids = nextTagOrder(dragged.id, spot.tagId, spot.before);
    if (ids) persistTagOrder(ids).catch((e) => setStatus(e.message, true));
  });

  list.addEventListener('dragend', () => {
    tagDragState = null;
    clearTagDropHint();
    for (const li of el('tags').children) li.classList.remove('dragging');
  });
}

// ---- 右键菜单：重命名 / 颜色（预设色板）/ 置顶切换 / 删除

/**
 * 标签项菜单。颜色走子菜单（既有 `openContextMenu` 的子菜单机制）：父项直出当前
 * 颜色名，子菜单列 8 个预设色 + 「默认」，当前项打勾。删除与其它项用分隔线隔开。
 */
function openTagMenu(ev, tag) {
  openContextMenu(ev, [
    { label: t('menu.rename'), action: () => renameTag(tag) },
    {
      label: `${t('menu.tagColor')} · ${tagColorLabel(tag)}`,
      submenu: () => [
        ...TAG_PALETTE.map((c) => ({
          label: t(c.key),
          checked: (tag.color || '').toLowerCase() === c.color,
          action: () => setTagColor(tag, c.color),
        })),
        { separator: true },
        {
          label: t('tags.color.none'),
          checked: !tag.color,
          action: () => setTagColor(tag, null),
        },
      ],
    },
    {
      label: tag.pinned ? t('menu.tagUnpin') : t('menu.tagPin'),
      action: () => toggleTagPinned(tag),
    },
    { separator: true },
    { label: t('menu.delete'), danger: true, action: () => deleteTag(tag) },
  ]);
}

/** 标签元信息变更后的统一收口：侧栏口径与选择器口径各重建一次（数量少，开销可忽略） */
async function afterTagMetaChange() {
  await refreshCounts(); // sidebar_data.tags：侧栏顺序 + 未读计数
  await refreshTagCache(); // 选择器口径：最近使用优先
}

/** 重命名：重名（core 判定，大小写不敏感）与空名都回可读错误，不静默吞 */
async function renameTag(tag) {
  const name = await promptText(t('tags.renameTitle'), tag.name);
  if (!name || name === tag.name) return;
  try {
    await invoke('rename_tag', { tagId: tag.id, name });
    await afterTagMetaChange();
    setStatus(t('tags.renamed', { name }));
    log(`tagRename: id=${tag.id} "${tag.name}"→"${name}" order=${tagOrderText()}`);
  } catch (e) {
    setStatus(e.message, true);
    log(`tagRename failed: id=${tag.id} "${tag.name}"→"${name}": ${e.message}`);
  }
}

/** 颜色：`null` = 清除回默认色（圆点用 --fg-dim） */
async function setTagColor(tag, color) {
  try {
    await invoke('set_tag_color', { tagId: tag.id, color });
    await afterTagMetaChange();
    setStatus(t('tags.colorSet', { name: tag.name }));
    log(`tagColor: id=${tag.id} name=${tag.name} color=${color || 'none'} order=${tagOrderText()}`);
  } catch (e) {
    setStatus(e.message, true);
    log(`tagColor failed: id=${tag.id} color=${color || 'none'}: ${e.message}`);
  }
}

/** 置顶切换：置顶的标签在侧栏排最前（顺序口径在 core，前端只重渲染） */
async function toggleTagPinned(tag) {
  try {
    const row = await invoke('set_tag_pinned', { tagId: tag.id, pinned: !tag.pinned });
    await afterTagMetaChange();
    setStatus(t(row.pinned ? 'tags.pinned' : 'tags.unpinned', { name: row.name }));
    log(`tagPin: id=${tag.id} name=${row.name} pinned=${row.pinned ? 1 : 0} order=${tagOrderText()}`);
  } catch (e) {
    setStatus(e.message, true);
    log(`tagPin failed: id=${tag.id}: ${e.message}`);
  }
}

/** 删除后清内存里的标签痕迹（含所有条目对象上的 chips 与阅读器 chips 的局部 patch） */
function dropTagFromMemory(tagId) {
  state.tags = (state.tags || []).filter((tg) => tg.id !== tagId);
  state.sidebarTags = (state.sidebarTags || []).filter((tg) => tg.id !== tagId);
  for (const e of state.entries) {
    if (e.tags) e.tags = e.tags.filter((x) => x.id !== tagId);
  }
  if (state.readerEntry && state.readerEntry.tags) {
    state.readerEntry.tags = state.readerEntry.tags.filter((x) => x.id !== tagId);
    patchReaderTags();
  }
}

/**
 * 删除标签：先 `dry_run` 拿影响篇数（与执行同源的计数函数）→ 通用确认弹窗展示 →
 * 确认后才真删。删除只清 `entry_tags` 关联，**文章保留**。
 */
async function deleteTag(tag) {
  let preview;
  try {
    preview = await invoke('delete_tag', { tagId: tag.id, dryRun: true });
  } catch (e) {
    setStatus(e.message, true);
    log(`tagDelete preview failed: id=${tag.id}: ${e.message}`);
    return;
  }
  const confirmed = await confirmDialog({
    title: t('tags.deleteTitle'),
    body: t('tags.deleteBody', { name: tag.name, n: preview.affected_entries }),
    okLabel: t('menu.delete'),
  });
  log(`tagDelete: preview id=${tag.id} name=${tag.name} affected=${preview.affected_entries} confirmed=${confirmed ? 1 : 0}`);
  if (!confirmed) return;
  try {
    const report = await invoke('delete_tag', { tagId: tag.id, dryRun: false });
    const wasCurrentView = state.view.kind === 'tag' && state.view.tagId === tag.id;
    dropTagFromMemory(tag.id);
    // 当前正看这个标签 → 退回未读（不然停在已不存在的筛选上）；否则静默重拉列表，
    // 让行上的 chips 去掉这个标签（阅读区与滚动位置不动）。
    if (wasCurrentView) await setView(VIEWS.find((v) => v.kind === 'unread'));
    else await loadEntries({ reader: false });
    await afterTagMetaChange();
    setStatus(t('tags.deleted', { name: tag.name, n: report.affected_entries }));
    log(
      `tagDelete: done id=${tag.id} name=${tag.name} affected=${report.affected_entries} ` +
        `view=${state.view.kind} rows=${state.entries.length} head=${headIds()} order=${tagOrderText()}`
    );
  } catch (e) {
    setStatus(e.message, true);
    log(`tagDelete failed: id=${tag.id}: ${e.message}`);
  }
}

// ---------------------------------------------------------------- 数据流

/// 全量读数据并渲染。
/// `reader: false` 是后台刷新用的静默模式：只重读侧栏与列表，不重渲染正文——
/// 阅读焦点与滚动位置保持原位（定时刷新到点时用户可能正在读一篇长文）。
async function loadAll({ reader = true } = {}) {
  const [sidebar, settings, ai, mcp, collapsed, tagsCollapsed, tags] = await Promise.all([
    invoke('sidebar_data'),
    invoke('get_ui_settings'),
    invoke('get_ai_settings'),
    invoke('get_mcp_settings'),
    invoke('get_collapsed_folders'),
    invoke('get_tags_collapsed'),
    // 选择器顺序（最近使用优先）由 core 直出：前端不排序，只存
    invoke('list_tags', { recentFirst: true }),
  ]);
  state.db = sidebar.db;
  state.feeds = sidebar.feeds;
  state.folders = sidebar.folders;
  // 侧栏标签区用的是同一批 TagRow 的侧栏口径（置顶优先）
  state.sidebarTags = sidebar.tags || [];
  state.collapsedFolders = collapsed;
  state.tagsCollapsed = !!tagsCollapsed;
  acceptSettings(settings);
  state.ai = ai;
  state.mcp = mcp;
  state.tags = tags;
  // 语言设置来自数据库；先应用再渲染，避免先闪一下默认语言
  setLocale(settings.locale || 'auto');
  // 主题同理：渲染前先设好 data-theme，避免启动时闪错色
  applyTheme();
  bindSettingDropdowns();
  applyStaticI18n();
  renderSidebar();
  await loadEntries({ reader });
  log(
    `loaded feeds=${sidebar.db.feeds} entries=${sidebar.db.entries} unread=${sidebar.db.unread} starred=${sidebar.db.starred} markReadOnNavigate=${settings.mark_read_on_navigate} refreshInterval=${settings.refresh_interval_minutes} refreshOnStart=${settings.refresh_on_start} notifyNewArticles=${settings.notify_new_articles} fonts ui=${settings.font_ui || 'default'} read=${settings.font_read || 'follow-ui'} mono=${settings.font_mono || 'default'} size=${fontSizeText(settings.font_read_size)}px line=${fontLineText(settings.font_read_line)} logLevel=${settings.log_level} ai=${ai.provider}${ai.model ? '/' + ai.model : '（未配模型）'} hasKey=${ai.has_key} mcp=${mcp.running ? mcp.url : 'off'} mcpWrite=${mcp.write_enabled ? 'on' : 'off'} mcpDangerous=${mcp.dangerous_enabled ? 'on' : 'off'} mcpWriteToken=${mcp.write_token ? 'set' : 'none'}${reader ? '' : ' silent（正文未重渲染）'}`
  );
}

/// 每批条数：与后端 list_entries 的默认值一致，也是「有没有下一页」的判据。
const PAGE_SIZE = 200;

/// 列表排序档（`list.sort` 设置）。缺省或未加载时按默认档 'newest' 处理
/// ——与 Rust 侧 `ListSort::from_setting` 的白名单口径一致（非法值回默认档）。
function listSortMode() {
  const v = state.settings && state.settings.list_sort;
  return v === 'oldest' || v === 'unread_first' ? v : 'newest';
}

/// 列表「隐藏已读」开关（`list.hide_read`）。星标 / 稍后读视图的豁免在 store 查询层，
/// 前端不做第二套判断。
function listHideRead() {
  return !!(state.settings && state.settings.list_hide_read);
}

/// 列表分页状态（keyset 复合游标）。
/// cursor 记的是「已取到的最后一行」而不是「当前列表最后一行」：未读视图里读完一篇
/// 会把它从列表里移除，若拿剩下的末行当游标，读空一整批之后就再也取不到后面的未读
/// 条目——游标是结果流里的位置，不随某行被移出列表而后退。
const paging = { cursor: null, exhausted: false, loading: false, error: false, generation: 0 };

/// 当前视图对应的 `list_entries` 参数（`cursor: null` = 取首页）。
/// 首页与续页必须用同一套筛选，抽出来避免后台刷新的 prepend 比对另抄一份漂移。
function listArgs(cursor) {
  const kind = state.view.kind;
  return {
    feedId: kind === 'feed' ? state.feedId : null,
    folderId: kind === 'folder' ? state.view.folderId : null,
    unreadOnly: kind === 'unread',
    starredOnly: kind === 'starred',
    readLaterOnly: kind === 'later',
    // 标签视图 = 按标签筛选的普通列表（与 feed 视图同档：排序/隐藏已读仍跟随设置）
    tagId: kind === 'tag' ? state.view.tagId : null,
    limit: PAGE_SIZE,
    // 游标 = 上一页末行的 (sortkey, id, read)；sortkey 由后端直出，前端不按
    // published_at/fetched_at 自己算（前端也拿不到 fetched_at）。read 分量只有
    // `unread_first` 档用得上（该档排序键是 (read, sortkey, id)），其余档传了也被忽略。
    cursorSortkey: cursor ? cursor.sortkey : null,
    cursorId: cursor ? cursor.id : null,
    cursorRead: cursor ? cursor.read : null,
  };
}

/// 取一页并记下游标与「有没有下一页」。判据是「返回不足一批」：满批也可能是最后一页，
/// 多请求一次空页的代价可以接受，换来的是不必猜。
async function listRequest(command, args, generation) {
  try {
    const rows = await invoke(command, args);
    return generation === paging.generation ? rows : null;
  } catch (e) {
    if (generation !== paging.generation) return null;
    throw e;
  }
}

async function loadPage(cursor) {
  const rows = await listRequest('list_entries', listArgs(cursor), paging.generation);
  if (rows === null) return null;
  const last = rows[rows.length - 1];
  if (last) paging.cursor = { sortkey: last.sortkey, id: last.id, read: last.read };
  paging.exhausted = rows.length < PAGE_SIZE;
  return rows;
}

/// 诊断用（只读）：列表视口顶部第一条可见行的 id。
/// 后台刷新前后各取一次，两次相同才真正证明「看到的内容没被新条目顶走」——
/// scrollTop 相等只是必要条件，行高不一致时它并不充分。
function topVisibleRowId(list) {
  const rows = [...list.children].filter((li) => li.dataset.id);
  if (!rows.length) return 'none';
  const base = rows[0].offsetTop;
  const top = list.scrollTop;
  for (const li of rows) {
    if (li.offsetTop + li.offsetHeight - base > top) return li.dataset.id;
  }
  return 'none';
}

/// 后台刷新时把「比已加载首行更新」的条目插到列表最前面，并保持滚动态的视口不动。
/// 只在多页已加载时调用（单页/首屏列表本来就该从新行开始，走 reset 路径）。
/// 只动头部：append 游标、exhausted、尾部哨兵都不受影响（新条目只会让总数更多）；
/// 已加载的行一个都不重建——重建会把滚动位置冲掉，也是打开文章时的 CPU 尖峰来源。
async function prependFreshEntries() {
  // 排序档门控：只有 newest 档的「更新」才在列表头部。oldest 档的新条目属于列表
  // **尾部**、unread_first 档的新未读条目属于未读组头部（在已读组之前）——这两档把
  // 新条目插到 DOM 头部都会让 DOM 顺序与后端顺序不一致（挤在头部的最新行会让
  // 「最早在前」的列表实际上从最新开始），所以一律不走 prepend。
  // 非 newest 档退化为静默 reset（loadEntries 的 reset 路径：整体重查重渲、分页状态
  // 重置）。**滚动保持降级**：reset 重建全部行，只保留 scrollTop 像素偏移、不做锚点
  // 补偿——新条目落在头部时（unread_first 的新未读）可见的那几行会整体位移。这是
  // 文档化的取舍：换来列表顺序始终与排序档一致（顺序错更隐蔽也更糟）。
  // 实测（Xvfb + 260 条库，见验证清单）：oldest 档 listScrollTop=15676→15676
  // （新条目在尾部，视觉上不动）；unread_first 档同样是像素偏移保持、内容整体位移。
  if (listSortMode() !== 'newest') {
    const scrollBefore = el('entries').scrollTop;
    await loadEntries({ reader: false });
    log(
      `refresh:done prepend skipped（sort=${listSortMode()}）：改走静默 reset，listScrollTop=${scrollBefore}→${el('entries').scrollTop}（reset 重建，无滚动锚点补偿）`
    );
    return;
  }
  const first = state.entries[0];
  const generation = paging.generation;
  const rows = await listRequest('list_entries', listArgs(null), generation);
  if (rows === null || generation !== paging.generation) return;
  // 在飞期间用户可能换了视图/筛选或手动刷新（列表已被整体重建）：这次的结果作废，
  // 否则会把旧筛选下的条目插进新列表。重建出来的是新对象，用对象标识就能认出来；
  // 续页 append 不动头部，所以不影响判据。
  if (state.entries[0] !== first) {
    log(`refresh:done prepend skipped（列表在飞期间被重建）total=${state.entries.length}`);
    return;
  }
  const newer = rows.filter(
    (r) => r.sortkey > first.sortkey || (r.sortkey === first.sortkey && r.id > first.id)
  );
  // 会话内已读的不回插：未读视图里读过的那行已从列表删掉，但刷新查询可能先于
  // set_read 提交（竞态），服务端那条 unread 过滤这时还认为它是未读。
  const fresh = newer.filter((r) => !state.readSessionIds.has(r.id));
  const skipped = newer.length - fresh.length;
  const list = el('entries');
  const scrollBefore = list.scrollTop;
  const visibleBefore = topVisibleRowId(list);
  if (!fresh.length) {
    log(
      `refresh:done prepend rows=0 sessionReadSkipped=${skipped} listScrollTop=${scrollBefore}→${list.scrollTop} top=${visibleBefore}→${topVisibleRowId(list)} head=${list.firstChild?.dataset.id ?? 'none'} children=${list.children.length} total=${state.entries.length}`
    );
    return;
  }
  // 在顶部（scrollTop === 0）不补偿：用户正看着顶部，新条目应当直接可见；
  // 滚动态才按 scrollHeight 增量把视口钉回原处（否则新行会把内容整体顶下去）。
  const atTop = scrollBefore === 0;
  const heightBefore = list.scrollHeight;
  let ref = list.firstChild;
  // 倒序（旧→新）插到「上一次插入的那行」之前：最新的排在最上，DOM 顺序与
  // state.entries 的后端顺序（sortkey DESC）完全一致——锚点固定在原首行上的话，
  // 每次 insertBefore 都落在同一位置，DOM 会变成升序（最旧的新条在最上）。
  for (let i = fresh.length - 1; i >= 0; i--) {
    const row = buildEntryRow(fresh[i]);
    list.insertBefore(row, ref);
    ref = row;
  }
  state.entries = fresh.concat(state.entries);
  if (!atTop) list.scrollTop += list.scrollHeight - heightBefore;
  invalidateViewTotal(); // 新条目入库：总数可能变了，下次渲染重查
  renderListCount();
  void fetchViewTotal();
  log(
    `refresh:done prepend rows=${fresh.length} ids=${fresh.map((e) => e.id).join(',')} sessionReadSkipped=${skipped} atTop=${atTop} listScrollTop=${scrollBefore}→${list.scrollTop} height=${heightBefore}→${list.scrollHeight} top=${visibleBefore}→${topVisibleRowId(list)} head=${list.firstChild?.dataset.id ?? 'none'} children=${list.children.length} total=${state.entries.length}`
  );
}

/// 列表头排序入口的菜单：三档（当前档打勾）+ 分隔线 + 隐藏已读开关。
/// 动作分两步：写库（`set_list_*` 命令回 UiSettings 的归一值）→ reset 重建列表。
function openListSortMenu(ev, anchor) {
  const cur = listSortMode();
  const pick = (value, label) => ({
    label,
    checked: cur === value,
    action: () => applyListSetting(() => invoke('set_list_sort', { sort: value })),
  });
  openContextMenu(
    ev,
    [
      pick('newest', t('list.sortNewest')),
      pick('oldest', t('list.sortOldest')),
      pick('unread_first', t('list.sortUnreadFirst')),
      { separator: true },
      {
        label: t('list.hideRead'),
        checked: listHideRead(),
        action: () =>
          applyListSetting(() => invoke('set_list_hide_read', { enabled: !listHideRead() })),
      },
    ],
    anchor
  );
}

/// 两个列表设置动作的公共收尾：落库（回显归一值）→ reset 重建列表（分页状态重置）。
/// `reader: false`：列表整体重查重渲，但阅读区与正文滚动位置不动——换个排序不该把
/// 用户正在读的那篇清空（换视图是 reader: true 的占位语义，两件事分开）。
async function applyListSetting(run) {
  try {
    acceptSettings(await run());
    await loadEntries({ reader: false });
    log(`list settings: sort=${listSortMode()} hideRead=${listHideRead() ? 1 : 0}`);
  } catch (err) {
    setStatus(err.message, true);
    log(`list settings failed: ${err.message}`);
  }
}

/// 视图切换后不再自动打开首篇：展示但不标读会误导（正文都渲染了列表却不变灰），
/// 自动标读又违背「点击才算已读」的预期（实测反馈 2026-09-22 两轮）。
/// 清空选中 + 阅读区占位；首次点击列表行或键盘 j/k 才打开并按设置标读。
function renderSelectedEntry() {
  state.selectedId = null;
  renderReaderEmpty();
}

/// 读数据并渲染列表。
/// - `reset: true`（默认）重建列表：换视图/换筛选/刷新走这条，分页状态一并重置；
/// - `reset: false` 续页：只把新一页 append 到列表尾部，已有 DOM 一个都不重建，
///   选中项与正文也都不动；
/// - `reader: false` 静默模式（后台刷新用）：只重读列表，正文与滚动位置保持原位。
async function loadEntries({ reader = true, reset = true } = {}) {
  const kind = state.view.kind;
  if (reset) {
    paging.generation++;
    paging.loading = false;
    paging.cursor = null;
    invalidateViewTotal();
  }
  const generation = paging.generation;

  // 搜索本版仍是一次性 200（PRD R3）：不装哨兵、不参与续页
  if (kind === 'search') {
    paging.cursor = null;
    paging.exhausted = true;
    paging.error = false;
    const rows = state.query
      ? await listRequest('search', { query: state.query, limit: PAGE_SIZE }, generation) : [];
    if (rows === null || generation !== paging.generation) return;
    state.entries = rows;
    renderList();
    if (reader) renderSelectedEntry();
    log(`view=${kind} count=${state.entries.length} exhausted=true`);
    return;
  }

  if (!reset && paging.cursor) {
    const rows = await loadPage(paging.cursor);
    if (rows === null || generation !== paging.generation) return;
    // 运行时自证：续页不该重放已加载的行。dup 两个成因要分开看：
    // ① newest/oldest 档游标只看时间键，dup>0 = 游标语义坏了（必须报警）；
    // ② unread_first 的复合游标看 (read,sortkey,id)，分页之间用户读了已加载行
    //    （read 0→1 挪到已读组）后，这些行会重新落进下一页的游标窗口——这是
    //    排序语义的固有漂移，不是游标 bug。两种情况都在前端按已见 ID 去重，
    //    列表永不出现重复行；日志保留 dup 计数供诊断（code-review B1 实例）。
    const seen = new Set(state.entries.map((e) => e.id));
    const dup = rows.filter((e) => seen.has(e.id));
    const fresh = rows.filter((e) => !seen.has(e.id));
    const list = el('entries');
    state.entries = state.entries.concat(fresh);
    for (const e of fresh) list.appendChild(buildEntryRow(e));
    renderListCount();
    installSentinel();
    log(
      `append rows=${rows.length} fresh=${fresh.length} total=${state.entries.length} dup=${dup.length}${dup.length && listSortMode() !== 'unread_first' ? '（游标异常，非 unread_first 档不该重复）' : ''} exhausted=${paging.exhausted} sentinel="${sentinel ? sentinel.textContent : '-'}"`
    );
    return;
  }

  paging.cursor = null;
  paging.exhausted = false;
  paging.error = false;
  invalidateViewTotal(); // 换视图 / 换筛选：总数缓存作废（renderList 里会按需重查）
  // 列表要整体重建（换视图/换筛选/手动刷新）：会话已读集合对应的「已删除行」没了，清空
  state.readSessionIds.clear();
  const rows = await loadPage(null);
  if (rows === null || generation !== paging.generation) return;
  state.entries = rows;
  // 静默刷新（reader=false）不动 selectedId：正文区一个 DOM 都不动，选中态也
  // 保持——否则重指到首行后，操作按钮（标已读/星标/稍后读）会作用于用户没在看的
  // 文章（实测 2026-09-22：后台刷新把 selected 挪到 9320 而正文还是 9135）。
  // 静默刷新（reader=false）不动 selectedId；视图切换（reader=true）也不再回退
  // 选中首行——保持「点击才算已读」，阅读区显示占位（见 renderSelectedEntry）。
  renderList();
  // 重建路径的机器可核对诊断：档位 / 过滤 + 头部 id 序列。换排序、换过滤、刷新后的
  // 「顺序对不对」不用只能盯着屏幕看——head 直接与库里的期望顺序对比即可。
  log(
    `view=${kind}${state.feedId ? '#' + state.feedId : ''}${kind === 'tag' ? '#tag=' + state.view.tagId : ''} sort=${listSortMode()} hideRead=${listHideRead() ? 1 : 0} count=${state.entries.length} loaded=${loadedCount()} total=${viewTotalSync() ?? '-'} totalKind=${unreadFilteredView() ? 'unread' : 'all'} header=${el('list-count').textContent} exhausted=${paging.exhausted} head=${headIds()}`
  );
  // 静默模式到此为止：正文区一个 DOM 都不动
  if (!reader) return;
  await renderSelectedEntry();
}

/// 列表头部前 5 行的 id（诊断用，见上面重建路径的日志）。
function headIds() {
  return state.entries
    .slice(0, 5)
    .map((e) => e.id)
    .join(',');
}

/// 续页（哨兵自动触发 / 尾部按钮手动触发）。防重入：同一时刻只允许一个请求在飞——
/// 滚动抖动会让哨兵连续触发，用同一个游标并发请求会把同一批行 append 两遍。
/// 失败即停：自动触发在失败态被拦（`paging.error`），只有手动点击才清掉它重试。
async function loadMore({ manual = false } = {}) {
  if (paging.loading || paging.exhausted || !paging.cursor) return;
  if (paging.error && !manual) return; // 自动触发不给后端制造重试风暴
  if (paging.error && manual) {
    paging.error = false;
    log(`loadMore manual retry view=${state.view.kind}`);
  }
  const kind = state.view.kind;
  const generation = paging.generation;
  paging.loading = true;
  refreshSentinelFooter();
  try {
    await loadEntries({ reader: false, reset: false });
  } catch (err) {
    if (generation !== paging.generation) return;
    paging.error = true;
    // 失败后留一个可点的重试按钮（不再静默消失——那会儿只能靠换视图恢复）
    installSentinel();
    setStatus(t('status.loadMoreFailed', { error: err.message }), true);
    log(`loadMore failed view=${kind}: ${err.message}`);
  } finally {
    if (generation === paging.generation) {
      paging.loading = false;
      refreshSentinelFooter();
    }
  }
}

/// 列表重建路径（换视图/搜索/手动刷新/后台静默刷新）会整体重拉数据，灰显行在那里
/// 自然离开，不需要补页逻辑（未读行还在时哨兵续页；读完且游标尽了则重建后自然
/// 落到「暂无未读」占位）。

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

/** 打开即已读的公共落地：set_read + 灰显该行 + 计数/按钮文案/侧栏刷新。
 *  列表点击（openEntry）与视图切换自动展示首篇（renderSelectedEntry）共用，
 *  保证两条路径的已读表现完全一致。 */
async function markViewedRead(id) {
  await invoke('set_read', { ids: [id], read: true });
  state.readSessionIds.add(id);
  const row = state.entries.find((e) => e.id === id);
  if (row) row.read = true;
  // 未读视图里读过的文章**灰显而非立即删行**：立即删行会把高亮/键盘锚点/操作
  // 按钮的目标一起挪到下一篇，而阅读区还停在刚点开的文章——三者互相脱钩
  // （实测 2026-09-22：高亮在下一篇、正文还是被点的这篇）。灰显让「显示 =
  // 高亮 = 操作目标」保持同一篇，行在下次列表重建时自然离开，零重建。
  markRowRead(id);
  if (unreadFilteredView()) {
    renderListCount();
  }
  // renderReader 用的是 set_read 前取的 entry：按钮文案会滞后一拍（已读却写着
  // 「标为已读」）。只改这一个按钮的文本，不重渲染整个阅读区。
  const readBtn = el('act-read');
  if (readBtn && state.selectedId === id) setText(readBtn, t('reader.markUnread'));
  // 计数刷新节流：连续快速阅读时合并为一次全量刷新（600ms 去抖）
  refreshCountsSoon();
}

let readerRequest = 0;

/** 打开某篇文章；markRead=true 表示这是用户主动打开的动作 */
async function openEntry(id, { markRead, follow = true } = {}) {
  const request = ++readerRequest;
  const generation = paging.generation;
  const entry = await invoke('get_entry', { id });
  if (!entry || request !== readerRequest || generation !== paging.generation) return;
  const changed = state.selectedId !== id;
  state.selectedId = id;
  if (changed) focusRow(id, { follow });
  renderReader(entry);

  if (markRead && !entry.read) {
    await markViewedRead(id);
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
  // 侧栏标签区（顺序 + 未读计数）与聚合计数同一把锁、同一次 IPC 回来
  state.sidebarTags = data.tags || [];
  renderSidebar();
  // 计数变了要同时刷新列表头与尾部（两处都读这些数字，见 T1/T3）：不重渲的话，
  // 侧栏未读已经变了、头部「共 N」与尾部进度还是旧值，要等下次列表重建才追上。
  // 头部与尾部都同值短路，未变零写入。
  renderListCount();
  refreshSentinelFooter();
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
  hideSubmenu();
  el('ctx-menu')?.remove();
}

/// 子菜单：同一时刻至多一个。收起走延时——父项与子菜单之间隔着 2px 缝隙，指针
/// 穿过去会先触发父项的 mouseleave，立即收会闪关（见 scheduleHideSubmenu）。
const SUBMENU_HIDE_MS = 150;
let ctxSubmenu = null;

/// 收起子菜单（父菜单关闭、父菜单滚动、悬停到另一父项、执行条目时都走这里）
function hideSubmenu() {
  if (!ctxSubmenu) return;
  clearTimeout(ctxSubmenu.timer);
  ctxSubmenu.el.remove();
  ctxSubmenu = null;
}

function cancelHideSubmenu() {
  if (ctxSubmenu) clearTimeout(ctxSubmenu.timer);
}

/// 延时收起：到点时指针仍在父项或子菜单上就不收。判据用 `:hover` 现查而不是
/// 自己记 mouseenter/mouseleave 配对——跨元素的配对顺序不可靠，这里只有「现在
/// 指针在不在里面」这一个事实。
function scheduleHideSubmenu() {
  if (!ctxSubmenu) return;
  clearTimeout(ctxSubmenu.timer);
  ctxSubmenu.timer = setTimeout(() => {
    if (!ctxSubmenu) return;
    if (ctxSubmenu.el.matches(':hover') || ctxSubmenu.parent.matches(':hover')) return;
    hideSubmenu();
  }, SUBMENU_HIDE_MS);
}

/// 打开父项的子菜单：先收掉别的（悬停到另一父项 = 切换）。`item.submenu()` 懒求值，
/// 每次展开重建——勾选态与文案取的是展开这一刻的值。
function openSubmenu(btn, item) {
  hideSubmenu();
  const sub = document.createElement('div');
  sub.className = 'ctx-menu ctx-submenu';
  renderMenuItems(sub, item.submenu());
  sub.addEventListener('mouseenter', cancelHideSubmenu);
  sub.addEventListener('mouseleave', scheduleHideSubmenu);
  document.body.appendChild(sub);
  const rect = placeSubmenu(sub, btn);
  ctxSubmenu = { el: sub, parent: btn, timer: 0 };
  // 定位诊断：右边缘翻转 / 底部钳位 / 内部滚动 / 是否出界都靠这一行机械核对，
  // 不必盯屏幕（headless 冒烟同样读它）
  log(
    `submenu "${item.label}" side=${rect.side} ` +
      `rect=${Math.round(rect.left)},${Math.round(rect.top)} ${sub.offsetWidth}x${sub.offsetHeight} ` +
      `viewport=${window.innerWidth}x${window.innerHeight} ` +
      `scroll=${sub.scrollHeight > sub.clientHeight ? 1 : 0} outofview=${menuOutOfView(sub) ? 1 : 0}`
  );
}

/// 菜单是否越出视口（诊断断言用；子菜单与父菜单都必须为 0）
function menuOutOfView(node) {
  const r = node.getBoundingClientRect();
  return r.left < 0 || r.top < 0 || r.right > window.innerWidth || r.bottom > window.innerHeight;
}

/// 父项事件：悬停即展开、移出延时收起、点击切换。
function bindSubmenuParent(btn, item) {
  btn.addEventListener('mouseenter', () => {
    // 已展开就什么都不做：重建会闪，也会把勾选态刷新成「没变过的状态」
    if (ctxSubmenu && ctxSubmenu.parent === btn) return;
    openSubmenu(btn, item);
  });
  btn.addEventListener('mouseleave', scheduleHideSubmenu);
  btn.addEventListener('click', () => {
    if (ctxSubmenu && ctxSubmenu.parent === btn) hideSubmenu();
    else openSubmenu(btn, item);
  });
}

/// 条目渲染：父菜单与子菜单共用同一个渲染器（子菜单就是同一种容器 + 定位），
/// 因此子菜单天然支持既有全部条目类型。
/// - `separator` 画一条分组分隔线；`header` 是小号灰字的分组标题（不可点）。
/// - `checked` 参与勾选组：勾中项前面打 ✓，未勾中项留同宽占位（标签对齐）。
/// - `submenu: () => items` 是父项：右侧 chevron，无 action（点击只展开/收起）。
function renderMenuItems(menu, items) {
  for (const item of items) {
    if (item.separator) {
      const sep = document.createElement('div');
      sep.className = 'ctx-sep';
      menu.appendChild(sep);
      continue;
    }
    if (item.header) {
      const head = document.createElement('div');
      head.className = 'ctx-head';
      head.textContent = item.label;
      menu.appendChild(head);
      continue;
    }
    const btn = document.createElement('button');
    btn.type = 'button';
    if (item.danger) btn.classList.add('danger');
    if (item.checked !== undefined) {
      const mark = document.createElement('span');
      mark.className = 'mark';
      mark.textContent = item.checked ? '✓' : '';
      const label = document.createElement('span');
      label.className = 'label';
      label.textContent = item.label;
      btn.append(mark, label);
      if (item.checked) btn.classList.add('checked');
    } else if (item.submenu) {
      // 父项标签必须是独立 span：chevron 与小字缩略靠 flex 排开（见 .has-submenu）
      const label = document.createElement('span');
      label.className = 'label';
      label.textContent = item.label;
      btn.appendChild(label);
    } else {
      btn.textContent = item.label;
    }
    if (item.submenu) {
      btn.classList.add('has-submenu');
      bindSubmenuParent(btn, item);
    } else {
      btn.onclick = () => {
        closeContextMenu();
        item.action();
      };
    }
    menu.appendChild(btn);
  }
}

/// 通用右键菜单：items = [{label, danger?, action} | {separator: true} | {header: true, label}
///                          | {label, submenu: () => items}]
/// 容器用 `.ctx-menu` class 而非 `#ctx-menu` id：子菜单需要同一套样式却挂在 body 上
/// （不是父菜单的子节点），id 只有一个，样式必须走 class。
function openContextMenu(ev, items, anchor) {
  closeContextMenu();
  const menu = document.createElement('div');
  menu.id = 'ctx-menu';
  menu.className = 'ctx-menu';
  renderMenuItems(menu, items);
  // 父菜单自身滚动后子菜单会错位（子菜单贴的是父项行的实时矩形，不是父菜单容器）
  // → 滚动即收起，不做跟随：简单且不可能错位。子菜单自己的滚动不冒泡到这里。
  menu.addEventListener('scroll', hideSubmenu);
  document.body.appendChild(menu);
  // 锚定模式（设置页下拉）：宽度不小于触发按钮，展开点贴按钮下沿
  if (anchor) {
    const r = anchor.getBoundingClientRect();
    menu.style.minWidth = r.width + 'px';
    placeMenu(menu, r.left, r.bottom + 2);
  } else {
    placeMenu(menu, ev.clientX, ev.clientY);
  }
}

/// 菜单定位三步：优先贴锚点向下展开，高度压缩到可用空间（菜单内部滚动，
/// 它本来就 overflow-y:auto）；下方连 160px 都放不下时翻转到锚点上方；两边都
/// 放不下才贴顶。top/left 永不为负。
/// 背景：菜单 CSS 上限 460px，在视口底部右键时旧公式
/// `min(clientY, innerHeight - h - pad)` 会把 top 钳成极小值甚至负数——菜单
/// 飞到屏幕顶端，视觉上像挂在别的行上（实测 2026-09-22：以为在操作 ZZ 行，
/// 实际菜单属于另一行，破坏性操作误认风险）。
function placeMenu(menu, x, y) {
  const pad = 8;
  const MIN_USABLE = 160;
  menu.style.left =
    Math.max(pad, Math.min(x, window.innerWidth - menu.offsetWidth - pad)) + 'px';
  const below = window.innerHeight - y - pad;
  const above = y - pad;
  if (below >= MIN_USABLE) {
    // 贴锚点向下：高度收到可用空间（内联值覆盖 CSS 的 min(70vh,460)）
    menu.style.maxHeight = Math.min(460, below) + 'px';
    menu.style.top = y + 'px';
  } else if (above >= MIN_USABLE) {
    // 翻转到锚点上方：先限高再量实际高度，top 用收紧后的 offsetHeight
    menu.style.maxHeight = Math.min(460, above) + 'px';
    menu.style.top = Math.max(pad, y - menu.offsetHeight - 2) + 'px';
  } else {
    // 两边都放不下（窗口太矮）：清掉内联限高回 CSS 规则，贴顶整高
    menu.style.maxHeight = '';
    menu.style.top = pad + 'px';
  }
}

/// 子菜单矩形计算（纯函数，boot 自检覆盖；不碰 DOM 便于用例穷举）：
/// - 水平：默认贴父项行右缘 +2；右侧放不下翻到父项行左侧 -2；两侧都紧时钳到 pad。
/// - 垂直：钳位 [pad, 视口高 - 高度 - pad]，高度超上限则限高（容器本来就 overflow-y:auto，
///   于是变成内部滚动）。
/// `side` 只记录「翻没翻」，`left/top` 是钳位后的最终值。
function submenuRect(row, size, viewport, pad = 8) {
  const maxH = Math.min(460, viewport.h - pad * 2);
  const h = Math.min(size.h, maxH);
  let side = 'right';
  let left = row.right + 2;
  if (left + size.w > viewport.w - pad) {
    side = 'left';
    left = row.left - size.w - 2;
  }
  return {
    side,
    maxH,
    left: Math.max(pad, Math.min(left, viewport.w - size.w - pad)),
    top: Math.max(pad, Math.min(row.top, viewport.h - h - pad)),
  };
}

/// 子菜单定位：贴**父项行**的实时矩形（不是父菜单容器）——父菜单自身可滚动，
/// 贴容器会在滚动后错位。调用前子菜单必须已在文档里，否则量不到尺寸。
function placeSubmenu(sub, parentRow) {
  const row = parentRow.getBoundingClientRect();
  const viewport = { w: window.innerWidth, h: window.innerHeight };
  // 宽度上限：CSS 已限死 260，这里只在视口更窄时兜底（保证绝不比视口宽）
  sub.style.maxWidth = Math.min(260, viewport.w - 16) + 'px';
  const rect = submenuRect(row, { w: sub.offsetWidth, h: sub.offsetHeight }, viewport);
  sub.style.maxHeight = rect.maxH + 'px';
  sub.style.left = rect.left + 'px';
  sub.style.top = rect.top + 'px';
  sub.dataset.side = rect.side;
  return rect;
}

/// 每源刷新间隔档位：`value` 与后端 `set_feed_refresh_interval` 的白名单一一对应
/// （`null` / `"global"` = 跟随全局档）。单源没有「关闭」档——要停自动刷新就选
/// 「跟随全局」并把全局档关掉，避免出现两套「关」的语义。
/// 档位文案直接复用设置页的 `settings.refreshMin*`：同一档位在设置页与菜单里
/// 必须是同一句话（tech_design 允许复用或新建 key，这里选复用以免两份翻译漂移）。
const FEED_REFRESH_CHOICES = [
  { value: null, label: followGlobalLabel },
  { value: '15', label: () => t('settings.refreshMin15') },
  { value: '30', label: () => t('settings.refreshMin30') },
  { value: '60', label: () => t('settings.refreshMin60') },
  { value: '120', label: () => t('settings.refreshMin120') },
  { value: '360', label: () => t('settings.refreshMin360') },
];

/// 档位（分钟数，来自 FeedRow）→ 界面文案；认不出的值原样显示数字，不静默换档
function feedIntervalLabel(minutes) {
  const choice = FEED_REFRESH_CHOICES.find((c) => c.value === String(minutes));
  return choice ? choice.label() : String(minutes);
}

/// 「跟随全局」项的口径：跟随的就是全局档，全局关掉时这个源也随之不自动刷新
/// （评审 B1 的例外只针对**已单独设置间隔**的源，所以这里必须把当前全局档写出来，
/// 否则用户分不清「跟随」到底跟到了什么）。
function followGlobalLabel() {
  const minutes = state.settings?.refresh_interval_minutes;
  const globalText =
    !minutes || minutes === 'off' ? t('settings.refreshOff') : t(`settings.refreshMin${minutes}`);
  return t('menu.refreshFollowGlobalWith', { state: globalText });
}

function openFeedMenu(ev, feed) {
  const items = [];
  const siblings = state.feeds.filter(f => f.folder_id === feed.folder_id);
  const index = siblings.findIndex(f => f.id === feed.id);
  if (index > 0) items.push({ label: t('menu.feedUp'), action: () => moveFeed(feed.id, siblings[index - 1].id, true) });
  if (index >= 0 && index < siblings.length - 1) items.push({ label: t('menu.feedDown'), action: () => moveFeed(feed.id, siblings[index + 1].id, false) });
  // 立即刷新置顶：原先只能双击源标题触发（可发现性差，实测用户不知道）；
  // 与移动/刷新间隔组用分隔线隔开
  items.push({ label: t('menu.refreshNow'), action: () => refreshOne(feed.id) });
  items.push({ separator: true });
  // 编辑：标题/文件夹/间隔三件套的收敛入口（与下面的快捷项同一落库路径）
  items.push({ label: t('menu.editFeed'), action: () => openFeedEditDialog(feed) });
  // 刷新间隔：父项直出当前档位（FeedRow 的 refresh_interval_minutes），子菜单 6 档打勾。
  // 原先 6 个档位平铺在菜单里，21 个分组 + 6 档 = 菜单本体几十行，找一项要滚半天。
  const current =
    feed.refresh_interval_minutes == null ? null : String(feed.refresh_interval_minutes);
  const currentChoice = FEED_REFRESH_CHOICES.find((c) => c.value === current);
  items.push({
    label: `${t('menu.refreshInterval')} · ${
      currentChoice ? currentChoice.label() : feedIntervalLabel(feed.refresh_interval_minutes)
    }`,
    submenu: () =>
      FEED_REFRESH_CHOICES.map((choice) => ({
        label: choice.label(),
        checked: current === choice.value,
        action: () => setFeedRefreshInterval(feed.id, choice.value),
      })),
  });
  // 移动到：父项直出当前分组（未分组时写「未分组」），子菜单列出全部分组 + 未分组项、
  // 当前所在项打勾。分组列表懒求值：列表可能因新建/删除分组而变。
  const currentFolder = (state.folders || []).find((folder) => folder.id === feed.folder_id);
  items.push({
    label: `${t('menu.moveTo')} · ${currentFolder ? currentFolder.name : t('menu.ungrouped')}`,
    submenu: () => [
      ...(state.folders || []).map((folder) => ({
        label: folder.name,
        checked: folder.id === feed.folder_id,
        action: () => reassignFeed(feed.id, folder.id),
      })),
      {
        label: t('menu.ungrouped'),
        checked: feed.folder_id == null,
        action: () => reassignFeed(feed.id, null),
      },
    ],
  });
  // 取消订阅：破坏性（条目级联删），与上面的管理项分隔开，红色危险样式
  items.push({ separator: true });
  items.push({
    label: t('menu.unsubscribe'),
    danger: true,
    action: () => unsubscribeFeed(feed),
  });
  openContextMenu(ev, items);
}

/// 保存单源刷新间隔覆盖；返回的行已带最新 refresh_interval_minutes，但侧栏勾选态/
/// tooltip 与计数从同一条 `sidebar_data` 读取路径更新，避免两套状态漂移。
function setFeedRefreshInterval(feedId, value) {
  invoke('set_feed_refresh_interval', { feedId, value })
    .then((row) => {
      log(`feed ${feedId} refreshInterval=${row.refresh_interval_minutes ?? 'global'}`);
      return refreshCounts();
    })
    .catch((err) => {
      setStatus(err.message, true);
      log(`set_feed_refresh_interval failed: ${err.message}`);
    });
}

/// 通用危险操作确认弹窗：返回 Promise<boolean>（true=确认）。Esc/点遮罩/取消均关闭。
/// 复用 confirm-sheet 样式（小尺寸变体），与 AI 确认弹窗同一套视觉。
function confirmDialog({ title, body, okLabel, cancelLabel, danger = true }) {
  return new Promise((resolve) => {
    const overlay = el('generic-confirm-overlay');
    el('generic-confirm-title').textContent = title;
    el('generic-confirm-body').textContent = body || '';
    const ok = el('generic-confirm-ok');
    const cancel = el('generic-confirm-cancel');
    ok.textContent = okLabel || t('confirm.ok');
    cancel.textContent = cancelLabel || t('confirm.cancel');
    ok.classList.toggle('danger', danger);
    const done = (v) => {
      overlay.classList.add('hidden');
      ok.onclick = cancel.onclick = overlay.onclick = null;
      document.removeEventListener('keydown', onKey);
      resolve(v);
    };
    ok.onclick = () => done(true);
    cancel.onclick = () => done(false);
    overlay.onclick = (ev) => { if (ev.target === overlay) done(false); };
    const onKey = (ev) => { if (ev.key === 'Escape') done(false); };
    document.addEventListener('keydown', onKey);
    overlay.classList.remove('hidden');
    ok.focus();
  });
}

/// 取消订阅：破坏性操作（条目级联删除，不可恢复），必须过确认弹窗。
async function unsubscribeFeed(feed) {
  const ok = await confirmDialog({
    title: t('confirm.unsubscribeTitle'),
    body: t('confirm.unsubscribeBody', { name: feed.title }),
    okLabel: t('confirm.unsubscribeOk'),
  });
  if (!ok) return;
  try {
    await invoke('remove_feed', { feedId: feed.id });
    // 当前正在看这个源的文章列表 → 回到「全部」避免空列表死状态
    if (state.view.kind === 'feed' && state.feedId === feed.id) {
      await setView(VIEWS.find((v) => v.kind === 'all'));
    } else {
      await refreshCounts();
      // 其他视图（全部未读/全部等）也要重拉：已删源的条目级联没了，但内存里的
      // 列表 DOM 还留着旧行，用户会看到「源删了文章还在」（实测 2026-09-22）。
      // 静默模式：侧栏计数 + 列表重建，正文与滚动保持原位。
      await loadEntries({ reader: false });
    }
    setStatus(t('status.unsubscribed', { name: feed.title }));
    log(`feed removed: ${feed.id} ${feed.title}`);
  } catch (err) {
    setStatus(err.message, true);
    log(`remove_feed failed: ${err.message}`);
  }
}

function openFolderMenu(ev, folder) {
  openContextMenu(ev, [
    { label: t('menu.rename'), action: () => renameFolder(folder) },
    { label: t('menu.delete'), danger: true, action: () => deleteFolder(folder) },
  ]);
}

async function reassignFeed(feedId, folderId) {
  try {
    await invoke('assign_feed_folder', { feedId, folderId });
    await refreshCounts();
    if (state.view.kind === 'folder') await loadEntries({ reader: false });
  } catch (err) {
    setStatus(err.message, true);
  }
}

// ---------------- 订阅源编辑对话框 ----------------
// 三字段先在本地暂存，点「保存」才一次落库：取消 / Esc / 点遮罩都不产生任何写入。
// 侧栏与右键菜单的快捷项仍旧保留，两条路径共用同一套归一化与落库（set_feed_config
// 复用 assign_folder 与刷新间隔白名单）。

/// 当前正在编辑的暂存值；`null` = 对话框未打开
let feedEdit = null;

function feedEditIntervalValue(feed) {
  return feed.refresh_interval_minutes == null ? null : String(feed.refresh_interval_minutes);
}

/// 源间隔值 → 界面文案（`null` = 跟随全局；档位文案与右键菜单同源）
function feedIntervalText(value) {
  return value == null ? followGlobalLabel() : feedIntervalLabel(Number(value));
}

/// 对话框字段描述：id / 选项 / 当前值 / 写入暂存 / 触发按钮文案
function feedEditField(kind) {
  if (kind === 'folder') {
    return {
      id: 'feed-edit-folder',
      choices: () => [
        { value: null, label: t('feedEdit.ungrouped') },
        ...(state.folders || []).map((f) => ({ value: f.id, label: f.name })),
      ],
      current: () => feedEdit.folderId,
      set: (v) => {
        feedEdit.folderId = v;
      },
      text: () => {
        if (feedEdit.folderId == null) return t('feedEdit.ungrouped');
        const folder = (state.folders || []).find((f) => f.id === feedEdit.folderId);
        return folder ? folder.name : String(feedEdit.folderId);
      },
    };
  }
  return {
    id: 'feed-edit-interval',
    choices: () => FEED_REFRESH_CHOICES.map((c) => ({ value: c.value, label: c.label() })),
    current: () => feedEdit.interval,
    set: (v) => {
      feedEdit.interval = v;
    },
    text: () => feedIntervalText(feedEdit.interval),
  };
}

/// 打开编辑对话框：用 FeedRow 的现有值预填（源站名当 placeholder，
/// 让用户随时看得见「清空输入框会显示成什么」）
function openFeedEditDialog(feed) {
  feedEdit = {
    feed,
    name: feed.custom_title || '',
    folderId: feed.folder_id ?? null,
    interval: feedEditIntervalValue(feed),
  };
  const input = el('feed-edit-name');
  input.value = feedEdit.name;
  input.placeholder = feed.source_title || feed.title;
  const url = el('feed-edit-url');
  url.textContent = feed.url;
  url.title = feed.url; // 长地址截断时悬停可看全（只读，不可改）
  paintFeedEditLabels();
  el('feed-edit-overlay').classList.remove('hidden');
  input.focus();
  input.select();
}

function paintFeedEditLabels() {
  for (const kind of ['folder', 'interval']) {
    const field = feedEditField(kind);
    setText(el(field.id), field.text());
  }
}

/// 关闭对话框：下拉菜单是挂在 body 上的，弹窗关了它不能留在屏幕上
function closeFeedEditDialog() {
  el('feed-edit-overlay').classList.add('hidden');
  closeContextMenu();
  feedEdit = null;
}

/// 打开某个字段的选项菜单：只改本地暂存值（不打库），「保存」才落库。
/// 触发按钮带 `.setting-dropdown` 类——boot 的「点菜单外面就关」监听只豁免这个
/// 类，否则开菜单的那一次点击会把它在同一事件里建了又删（设置页五个下拉踩过
/// 同一个坑，见 openContextMenu 的锚定模式）。
function toggleFeedEditDropdown(kind) {
  if (!feedEdit) return;
  const field = feedEditField(kind);
  const menu = el('ctx-menu');
  if (menu && menu.dataset.dropdown === field.id) {
    closeContextMenu();
    return;
  }
  const current = field.current();
  openContextMenu(
    { clientX: 0, clientY: 0 },
    field.choices().map((c) => ({
      label: c.label,
      checked: c.value === current,
      action: () => {
        field.set(c.value);
        paintFeedEditLabels();
      },
    })),
    el(field.id)
  );
  el('ctx-menu').dataset.dropdown = field.id;
}

/// 保存：只把**改过的**字段放进补丁（键缺省 = 后端不动那个字段）。
/// 清除自定义名 = 送空串（`null` 是「不动」，两者在协议层必须区分）。
/// 「跟随全局」同理送 `"global"` 而不是 `null`。
async function saveFeedEdit() {
  if (!feedEdit) return;
  const { feed } = feedEdit;
  const patch = {};
  const name = el('feed-edit-name').value.trim();
  if (name !== (feed.custom_title || '').trim()) patch.customTitle = name;
  if (feedEdit.folderId !== (feed.folder_id ?? null)) patch.folderId = feedEdit.folderId;
  if (feedEdit.interval !== feedEditIntervalValue(feed)) {
    patch.refreshInterval = feedEdit.interval ?? 'global';
  }
  closeFeedEditDialog();
  if (!Object.keys(patch).length) return; // 三字段都没改：不打库
  try {
    const row = await invoke('set_feed_config', { feedId: feed.id, patch });
    // 侧栏走 sidebar_data 重拉：改名后要按**新显示名**重排，本地就地改字段做不到重排
    await refreshCounts();
    syncFeedTitle(row);
    setStatus(t('status.feedSaved', { name: row.title }));
    log(
      `feed ${row.id} config saved name="${row.title}" source="${row.source_title}" folder=${
        row.folder_id ?? 'none'
      } interval=${row.refresh_interval_minutes ?? 'global'}`
    );
  } catch (err) {
    setStatus(err.message, true);
    log(`set_feed_config failed: ${err.message}`);
  }
}

/// 改名后同步已渲染的文本：列表行的源名 + 阅读区元信息。
/// 只改文本节点，不重建列表/正文（滚动位置、选中态、正文 DOM 都保持原位；
/// 侧栏由 refreshCounts → renderSidebar 的 keyed reconcile 负责）。
function syncFeedTitle(row) {
  for (const li of el('entries').children) {
    const entry = state.entries.find((e) => e.id === Number(li.dataset.id));
    if (!entry || entry.feed_id !== row.id) continue;
    entry.feed_title = row.title;
    const meta = li.querySelector('.meta span');
    if (meta) setText(meta, row.title);
  }
  if (state.readerFeedId === row.id) {
    const meta = document.querySelector('#reader .reader-head .meta span');
    if (meta) setText(meta, row.title);
  }
  if (state.view.kind === 'feed' && state.feedId === row.id) {
    el('list-title').textContent = viewTitle();
  }
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
    setText(el('list-title'), viewTitle());
  } catch (err) {
    setStatus(err.message, true);
  }
}

async function deleteFolder(folder) {
  try {
    await invoke('delete_folder', { folderId: folder.id });
    await refreshCounts();
    if (state.view.kind === 'folder' && state.view.folderId === folder.id) await setView({ kind: 'all' });
    log(`folder deleted: ${folder.name}`);
  } catch (err) {
    setStatus(err.message, true);
  }
}

function move(delta) {
  if (!state.entries.length) return;
  el('entries').focus({ preventScroll: true });
  const idx = state.entries.findIndex((e) => e.id === state.selectedId);
  const next = Math.max(0, Math.min(state.entries.length - 1, (idx < 0 ? 0 : idx) + delta));
  // j/k 是否顺便标已读完全取决于设置（默认开）
  openEntry(state.entries[next].id, { markRead: state.settings.mark_read_on_navigate });
}

function jump(toEnd) {
  if (!state.entries.length) return;
  el('entries').focus({ preventScroll: true });
  const target = toEnd ? state.entries[state.entries.length - 1] : state.entries[0];
  openEntry(target.id, { markRead: state.settings.mark_read_on_navigate });
}

async function toggleRead() {
  const row = toggleTarget();
  if (!row) return;
  const read = !row.read;
  await invoke('set_read', { ids: [row.id], read });
  if (read) state.readSessionIds.add(row.id);
  else state.readSessionIds.delete(row.id);
  setEntryFlag(row.id, 'read', read);
  // 未读视图同样只灰显不删行：按钮切换后留在原文章（不未经请求地跳到下一篇），
  // 双向都生效（读→灰、取消未读→恢复），计数口径同步更新。
  const li = rowEl(row.id);
  if (li) li.classList.toggle('read', read);
  // 阅读区只改动作按钮的文案：正文 DOM（含滚动位置与 AI 面板内容）一个字节都不动。
  // 先前这里走 renderReader 全量重建——renderReader 内 `reader.scrollTop = 0` 加上
  // 重建 `ai-panel`（初始 class=hidden、空 body），按 u 就是「正文跳回顶部 + 已生成的
  // 摘要消失」（审计 P1-2）。
  if (state.readerEntry && state.readerEntry.id === row.id) {
    const readBtn = el('act-read');
    if (readBtn) readBtn.textContent = read ? t('reader.markUnread') : t('reader.markRead');
  }
  logReaderState('toggleRead', row.id);
  if (unreadFilteredView()) {
    renderListCount();
  }
  await refreshCounts();
}

async function toggleStar() {
  const row = toggleTarget();
  if (!row) return;
  const starred = !row.starred;
  await invoke('set_starred', { ids: [row.id], starred });
  setEntryFlag(row.id, 'starred', starred);
  // 行级 patch：★ 标记就地增删（与 buildEntryRow 同一处结构），不重建列表——
  // renderList 会把列表滚动位置冲掉，也不该为一次标记重排 200 行 DOM。
  const li = rowEl(row.id);
  if (li) {
    const meta = li.querySelector('.meta');
    const star = meta.querySelector('.star');
    if (starred && !star) {
      const span = document.createElement('span');
      span.className = 'star';
      span.textContent = '★';
      meta.insertBefore(span, meta.querySelector('.later-mark'));
    } else if (!starred && star) {
      star.remove();
    }
  }
  // 阅读区只改按钮文案，正文 DOM 不动（同 toggleRead）
  if (state.readerEntry && state.readerEntry.id === row.id) {
    const starBtn = el('act-star');
    if (starBtn) starBtn.textContent = starred ? t('reader.removeStar') : t('reader.addStar');
  }
  // 星标视图里取消星标 → 该行不再属于本视图：定向移除该行（先前是 renderList 整表重建）
  if (state.view.kind === 'starred' && !starred) dropRowFromList(row.id);
  logReaderState('toggleStar', row.id);
  await refreshCounts();
}

/// 稍后读：与已读/星标独立；当前视图是稍后读时，取消标记要从列表移除该行。
/// `id` 由列表行的 ⚑ 点击显式传入（只切那一行）；缺省 = 当前选中的那行（按钮/快捷键）。
async function toggleReadLater(id = state.selectedId) {
  const row = toggleTarget(id);
  if (!row) return;
  const readLater = !row.read_later;
  await invoke('set_read_later', { ids: [row.id], readLater });
  setEntryFlag(row.id, 'read_later', readLater);
  // 行级 patch：⚑ 激活态就地切换，不重建列表
  const mark = rowEl(row.id)?.querySelector('.later-mark');
  if (mark) mark.classList.toggle('on', readLater);
  // 阅读区按钮只在「操作的就是阅读区正在展示的那篇」时才动：读 A 时给列表行 B 打 ⚑
  // 是合法操作，不能把 A 的按钮改成 B 的状态（阅读区本身同样一个 DOM 都不动）。
  if (state.readerEntry && state.readerEntry.id === row.id) {
    const laterBtn = el('act-later');
    if (laterBtn) {
      laterBtn.textContent = readLater ? t('reader.removeLater') : t('reader.markLater');
      laterBtn.classList.toggle('later-active', readLater);
    }
  }
  // 稍后读视图里取消标记 → 该行离开视图：定向移除（先前走 loadAll 整体重载）
  if (state.view.kind === 'later' && !readLater) dropRowFromList(row.id);
  logReaderState('toggleReadLater', row.id);
  await refreshCounts();
}

/**
 * 当前视图里「已不再属于本视图」的行：定向移除该行 + 同步 state.entries 与列表头计数，
 * 不整表重建（重建会把列表滚动位置冲掉，也是审计 P1-2 的同一类损耗）。正文与选中项都
 * 不动：用户正在读的那篇还留在屏幕上，再按一次 s/l 还能撤回来。
 * 行对象必须从 state.entries 摘掉：loadedCount 与后续整表重建都按它算，留着会多算一行。
 * 摘到空则交给 renderList 画空态（这时没有别的行可重建）。
 */
function dropRowFromList(id) {
  rowEl(id)?.remove();
  state.entries = state.entries.filter((e) => e.id !== id);
  log(`row dropped id=${id} view=${state.view.kind} rows=${state.entries.length}`);
  if (!state.entries.length) {
    renderList();
    return;
  }
  renderListCount();
}

async function doRefresh() {
  const btn = el('btn-refresh');
  btn.disabled = true;
  setStatus(t('status.refreshing'));
  try {
    // 并发档位从设置读（默认 6），与后台调度器同一条白名单口径
    const concurrency = state.settings.refresh_concurrency || 6;
    const r = await invoke('refresh_all', { concurrency });
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
/// done 分两支：多页已加载走 prepend（新条目插到头部 + 补偿滚动位置，见
/// `prependFreshEntries`），其余场景沿用静默 `loadAll({reader:false})`——
/// 两支都不重渲染正文，阅读焦点与正文滚动位置保持原位。
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
  // 逐源进度：把 start 时的笼统提示换成「N/M · 成功 X · 失败 Y」实时计数。
  // 事件无序到达（并发抓取），直接用 payload 里的快照值，不在前端累加。
  events
    .listen('refresh:progress', (e) => {
      const p = e.payload || {};
      backgroundRefreshHint = t('status.refreshProgress', {
        done: p.done ?? 0,
        total: p.total ?? 0,
        ok: p.ok ?? 0,
        failed: p.failed ?? 0,
      });
      setStatus(backgroundRefreshHint);
    })
    .catch((e) => log(`listen refresh:progress failed: ${e.message}`));
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
        // 多页已加载时只 prepend 新条目并补偿滚动位置（exhausted 不参与判据：小库/星标
        // 这类已耗尽的多页视图同样要保持位置）；续页在飞或上批失败时走 reset，避免与
        // loadMore 的游标/append 竞态。首屏/单页本来就没有位置要保，维持 reset 语义。
        if (state.entries.length > PAGE_SIZE && !paging.loading && !paging.error) {
          await prependFreshEntries();
          await refreshCounts(); // 侧栏计数照旧走现有路径；正文一个 DOM 都不动
        } else {
          await loadAll({ reader: false });
        }
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
    setStatus(t('status.discoverFailed', { error: fetchFailureMessage(e.code, e.message) }), true);
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
    if (r.failures.length) {
      setStatus(t('status.addedFetchFailed', { error: fetchFailureMessage(r.failures[0].code, r.failures[0].error) }), true);
      log(`add_feed initial fetch failed: ${r.failures[0].error}`);
    } else {
      setStatus(t('status.added', { inserted: r.inserted }));
    }
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

/**
 * 启动自检：菜单条目渲染契约。父菜单与子菜单共用一个渲染器，子菜单要能装下既有
 * 全部条目类型（普通 / 勾选 / 分组标题 / 分隔线），父项要带 chevron 且**没有 action**
 * （点击只展开/收起，不会误执行操作）。断言在游离节点上跑，不碰真实菜单。
 */
function selfTestMenuRender() {
  const box = document.createElement('div');
  renderMenuItems(box, [
    { label: 'plain', action: () => {} },
    { label: 'checked', checked: true, action: () => {} },
    { header: true, label: 'head' },
    { separator: true },
    { label: 'parent', submenu: () => [{ label: 'child', action: () => {} }] },
  ]);
  const failures = [];
  const buttons = box.querySelectorAll('button');
  if (buttons.length !== 3) failures.push(`button=${buttons.length}，期望 3`);
  if (!box.querySelector('.ctx-head')) failures.push('header 条目未渲染');
  if (!box.querySelector('.ctx-sep')) failures.push('separator 条目未渲染');
  const checked = box.querySelector('button.checked');
  if (!checked || checked.querySelector('.mark')?.textContent !== '✓') {
    failures.push('checked 条目未打勾');
  }
  const parent = box.querySelector('button.has-submenu');
  if (!parent) failures.push('父项未带 has-submenu（chevron 指示符）');
  else if (parent.onclick) failures.push('父项不应绑定 action 点击');
  log(
    failures.length
      ? `menu render selftest FAILED: ${failures.join('; ')}`
      : 'menu render selftest ok (items=5)'
  );
  return failures.length === 0;
}

/**
 * 启动自检：子菜单定位不变量。定位是纯函数 `submenuRect`，这里用合成矩形穷举
 * 翻转/钳位/限高三种分支，断言的是**不变量**（任何用例都不越出视口）而不是具体
 * 像素。右边缘翻转与底部钳位在真实界面里要靠窗口尺寸凑，这层自检把它们变成
 * 随时可核对的日志行（同 `selfTestSanitizer` 的口径）。
 */
function selfTestSubmenuPlacement() {
  const cases = [
    // 右侧有地方：直接贴父项行右侧，不动 vertical
    { name: 'right', row: { left: 100, right: 400, top: 300 }, size: { w: 200, h: 200 },
      viewport: { w: 1240, h: 820 }, side: 'right', top: 300, clamped: false },
    // 贴右边缘：翻转，落在父项行左侧
    { name: 'flip-left', row: { left: 1000, right: 1240, top: 300 }, size: { w: 200, h: 200 },
      viewport: { w: 1240, h: 820 }, side: 'left', top: 300, clamped: false },
    // 贴视口底部：垂直钳位（top 被压上来，bottom 恰好落在 pad 内）
    { name: 'bottom-clamp', row: { left: 100, right: 400, top: 780 }, size: { w: 200, h: 300 },
      viewport: { w: 1240, h: 820 }, side: 'right', top: 512, clamped: false },
    // 子菜单比可用高度还高：限高 + 内部滚动（maxH < 自然高）
    { name: 'scroll', row: { left: 100, right: 400, top: 300 }, size: { w: 200, h: 900 },
      viewport: { w: 1240, h: 820 }, side: 'right', top: 300, clamped: true },
    // 两侧都放不下（窄视口）：翻转后仍要钳回 pad，不越出
    { name: 'tight-both', row: { left: 40, right: 260, top: 300 }, size: { w: 200, h: 200 },
      viewport: { w: 300, h: 820 }, side: 'left', top: 300, clamped: false },
  ];
  const failures = [];
  for (const c of cases) {
    const r = submenuRect(c.row, c.size, c.viewport);
    const h = Math.min(c.size.h, r.maxH);
    if (r.side !== c.side) failures.push(`${c.name}: side=${r.side}，期望 ${c.side}`);
    if (r.top !== c.top) failures.push(`${c.name}: top=${r.top}，期望 ${c.top}`);
    if ((r.maxH < c.size.h) !== c.clamped) failures.push(`${c.name}: 限高不符期望`);
    // 硬不变量：四个边都不越出视口
    if (r.left < 8 || r.top < 8 || r.left + c.size.w > c.viewport.w - 8 || r.top + h > c.viewport.h - 8) {
      failures.push(`${c.name}: 越出视口 left=${r.left} top=${r.top} h=${h}`);
    }
  }
  log(
    failures.length
      ? `submenu placement selftest FAILED: ${failures.join('; ')}`
      : `submenu placement selftest ok (cases=${cases.length})`
  );
  return failures.length === 0;
}

async function markAll(read) {
  const { kind } = state.view;
  const scope = { kind };
  if (kind === 'feed') scope.id = state.view.feedId;
  if (kind === 'tag') scope.id = state.view.tagId;
  if (kind === 'folder') scope.id = state.view.folderId;
  if (kind === 'search') scope.query = state.query;
  const cmd = read ? 'mark_all_read' : 'mark_all_unread';
  try {
    const n = await invoke(cmd, { scope });
    // 批量标记同样算「会话内读过」：prepend 不回插（紧随其后的 loadAll 会重建列表并
    // 清空集合，这里跟上单条路径的语义，不让两条路径对不上）
    for (const e of state.entries) {
      if (read) state.readSessionIds.add(e.id);
      else state.readSessionIds.delete(e.id);
    }
    setStatus(t(read ? 'status.markedRead' : 'status.markedUnread', { n }));
    log(`${cmd} scope=${kind} changed=${n}`);
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

/// key 来源 → 当前语言的说明。后端只给结构化数据（keyring / env / unavailable），
/// 中文句子在这里出——en 界面里不会冒出中文括号与中文备注。
function keySourceText(src) {
  if (!src) return '';
  switch (src.kind) {
    case 'keyring':
      return t('settings.ai.keyFromKeyring');
    case 'env':
      return t('settings.ai.keyFromEnv', { name: src.name || 'RUSTSS_AI_KEY' });
    case 'unavailable':
      return t('settings.ai.keyUnavailable', { error: src.error || '' });
    default:
      return '';
  }
}

function fillAiForm() {
  const ai = state.ai;
  if (!ai) return;
  el('ai-provider').value = ai.provider;
  el('ai-model').value = ai.model;
  el('ai-base-url').value = ai.base_url;
  el('ai-target').value = ai.translate_target;
  el('ai-max-tokens').value = ai.max_output_tokens || 4096;
  el('ai-key').value = '';
  // key_source 两种含义都如实显示：已设置 → 来自哪里；未设置 → 为什么读不到
  // （读不到时保留「尚未设置；留空＝不修改」这句操作指引，原因作为后缀）
  const source = keySourceText(ai.key_source);
  const keyHint = ai.has_key
    ? t('settings.ai.keySet', { source })
    : t('settings.ai.keyUnset');
  el('ai-key-hint').textContent =
    !ai.has_key && source
      ? `${keyHint}${t('settings.ai.keyUnsetDetail', { reason: source })}`
      : keyHint;
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

  // 写能力：两个开关 + 写 token 三态（未生成 / 已生成）
  el('set-mcp-write-enabled').checked = !!mcp.write_enabled;
  el('set-mcp-dangerous-enabled').checked = !!mcp.dangerous_enabled;
  const hasWriteToken = !!mcp.write_token;
  el('mcp-write-token').textContent = hasWriteToken
    ? t('settings.mcp.writeTokenSet', {
        token: `${mcp.write_token.slice(0, 6)}…${mcp.write_token.slice(-4)}`,
      })
    : t('settings.mcp.writeTokenUnset');
  // 按钮与状态一一对应：没有 token 时只能「生成」（轮换/销毁/复制无意义）
  el('mcp-write-generate').classList.toggle('hidden', hasWriteToken);
  for (const id of ['mcp-write-copy', 'mcp-write-rotate', 'mcp-write-clear']) {
    el(id).classList.toggle('hidden', !hasWriteToken);
  }
  // 危险开关只有在写能力总开关打开后才有意义（关闭时灰置，避免“开了也没用”的困惑）
  el('set-mcp-dangerous-enabled').disabled = !mcp.write_enabled;
}

/** 切到某个设置分类（左栏可选，右栏只显示对应面板） */
function showPane(name) {
  for (const tab of el('settings-nav-list').querySelectorAll('[data-pane]')) {
    const active = tab.dataset.pane === name;
    tab.parentElement.classList.toggle('active', active);
    tab.setAttribute('aria-selected', String(active)); tab.tabIndex = active ? 0 : -1;
  }
  for (const pane of document.querySelectorAll('.settings-body .pane')) {
    pane.classList.toggle('hidden', pane.dataset.pane !== name);
  }
  document.querySelector('.settings-body').scrollTop = 0;
}

/// 记住上次看的是哪个分类（同一次运行内）
let currentPane = 'appearance';
let themeEditors = [];
let aaEditor;
let settingsReturnFocus;
function mountThemeEditor(id, kind) {
  return window.RustRssThemeSettings.createEditor(el(id), { kind,
    getSnapshot: () => state.settings.theme_snapshot, invoke, t, fontNames: () => state.fontFamilies,
    apply: settings => { acceptSettings(settings); applyTheme(); themeEditors.forEach(e => e.refresh()); },
  });
}
function closeSettings() {
  themeEditors.forEach(e => e.dispose()); themeEditors = [];
  el('settings-overlay').classList.add('hidden');
  settingsReturnFocus?.focus();
}
function openAa() {
  prefetchFontFamilies();
  aaEditor?.dispose(); aaEditor = mountThemeEditor('aa-editor', 'reading');
  el('aa-dialog').showModal();
}


/// 设置页自绘下拉：原生 <select> 的弹层由 GTK 系统主题绘制，应用内切深色它
/// 仍白底（实测 2026-09-22：Breeze 浅色弹层 #FFFFFF/#3DAEE9 混在深色 UI 里），
/// CSS 的 option 配色够不到它。改为按钮 + 锚定菜单（复用右键菜单基建），
/// 三态主题完全可控。注册表驱动：标签随 locale 重译、值随设置回填。
const SETTING_DROPDOWNS = [
  {
    id: 'set-language',
    choices: () => [
      { value: 'auto', label: t('settings.languageAuto') },
      { value: 'zh-CN', label: t('settings.languageZh') },
      { value: 'en', label: t('settings.languageEn') },
    ],
    current: () => state.settings.locale || 'auto',
    apply: (v) => {
      if (themeEditors.some(editor => editor.dirty)) throw new Error(t('theme.localeDraft'));
      return invoke('set_ui_locale', { locale: v });
    },
    after: async () => {
      setLocale(state.settings.locale || 'auto');
      applyStaticI18n();
      themeEditors.forEach(e => e.dispose());
      themeEditors = [mountThemeEditor('appearance-editor', 'appearance'), mountThemeEditor('reading-editor', 'reading')];
      renderSidebar();
      renderList();
      if (state.selectedId) {
        const entry = await invoke('get_entry', { id: state.selectedId });
        if (entry) renderReader(entry);
      }
    },
  },
  {
    id: 'set-close-action',
    choices: () => [
      { value: 'exit', label: t('settings.closeExit') },
      { value: 'tray', label: t('settings.closeTray') },
    ],
    current: () => state.settings.close_action || 'exit',
    apply: (v) => invoke('set_ui_close_action', { action: v }),
  },
  {
    id: 'set-refresh-interval',
    choices: () => [
      { value: 'off', label: t('settings.refreshOff') },
      { value: '15', label: t('settings.refreshMin15') },
      { value: '30', label: t('settings.refreshMin30') },
      { value: '60', label: t('settings.refreshMin60') },
      { value: '120', label: t('settings.refreshMin120') },
      { value: '360', label: t('settings.refreshMin360') },
    ],
    current: () => state.settings.refresh_interval_minutes || '30',
    apply: (v) => invoke('set_refresh_interval', { minutes: v }),
    after: () => {
      const minutes = state.settings.refresh_interval_minutes;
      setStatus(
        minutes === 'off'
          ? t('status.refreshIntervalOff')
          : t('status.refreshIntervalSet', { minutes })
      );
    },
  },
  {
    id: 'set-refresh-concurrency',
    choices: () =>
      [3, 6, 12, 24].map((n) => ({
        value: String(n),
        label: t('settings.refreshConcurrencyChoice', { n }),
      })),
    current: () => String(state.settings.refresh_concurrency || 6),
    apply: (v) => invoke('set_refresh_concurrency', { value: Number(v) }),
    after: () =>
      setStatus(
        t('status.refreshConcurrencySet', { n: state.settings.refresh_concurrency })
      ),
  },
  {
    // 思考强度：立即生效的独立旋钮（不经 AI 表单的保存按钮）；
    // 只在 OpenAI 兼容 provider 下随请求发送，其它 provider 选了也只是存着
    id: 'set-ai-reasoning',
    choices: () => [
      { value: '', label: t('settings.ai.reasoningAuto') },
      { value: 'minimal', label: t('settings.ai.reasoningMinimal') },
      { value: 'low', label: t('settings.ai.reasoningLow') },
      { value: 'medium', label: t('settings.ai.reasoningMedium') },
      { value: 'high', label: t('settings.ai.reasoningHigh') },
    ],
    current: () => (state.ai && state.ai.reasoning_effort) || '',
    // 基建会把返回值赋给 state.settings：这里返回的是 UiSettings 不变，
    // AI 视图单独存 state.ai
    apply: async (v) => {
      state.ai = await invoke('set_ai_reasoning_effort', { value: v });
      return state.settings;
    },
    after: () => setStatus(t('status.aiReasoningSet', { v: settingDropdownLabel(SETTING_DROPDOWNS.find((d) => d.id === 'set-ai-reasoning')) })),
  },
  {
    // 日志级别：写库后 Rust 侧立刻 set_max_level（即时生效，不用重启）。
    // debug 只收本应用（rustrss*/ui）的记录，依赖库的 debug 由 core writer 按 target 丢弃。
    id: 'set-log-level',
    choices: () => [
      { value: 'info', label: t('settings.logLevelInfo') },
      { value: 'debug', label: t('settings.logLevelDebug') },
    ],
    current: () => state.settings.log_level || 'info',
    apply: (v) => invoke('set_log_level', { level: v }),
    after: () =>
      setStatus(
        t('status.logLevelSet', {
          v: settingDropdownLabel(SETTING_DROPDOWNS.find((d) => d.id === 'set-log-level')),
        })
      ),
  },
];

function settingDropdownLabel(d) {
  const cur = d.current();
  return (d.choices().find((c) => c.value === cur) || d.choices()[0]).label;
}

/// 预取系统字体族：**只在第一次打开设置页时**跑（fc-list 是子进程，不进启动路径）；
/// 共享外观/阅读/Aa 编辑器读取此缓存；加载完成即刷新建议，失败后允许下次重试。
function prefetchFontFamilies() {
  if (state.fontFamiliesLoaded) return;
  state.fontFamiliesLoaded = true; // 先置位：多次打开不重复 spawn
  invoke('list_font_families')
    .then((families) => {
      state.fontFamilies = Array.isArray(families) ? families : [];
      themeEditors.forEach(editor => editor.refreshFonts());
      aaEditor?.refreshFonts();
      refreshSettingDropdowns();
      log(
        state.fontFamilies.length
          ? `font families=${state.fontFamilies.length}`
          : 'font families=0（只有「跟随主题」可选）'
      );
    })
    .catch((e) => {
      state.fontFamilies = [];
      state.fontFamiliesLoaded = false;
      log(`list_font_families failed: ${e.message}（字体下拉只有「跟随主题」）`);
    });
}

/// 打开/关闭某个下拉：再点同一下拉 = 关闭（菜单源标记防「关了叉开」）
function toggleSettingDropdown(d) {
  const menu = el('ctx-menu');
  if (menu && menu.dataset.dropdown === d.id) {
    closeContextMenu();
    return;
  }
  const cur = d.current();
  openContextMenu(
    { clientX: 0, clientY: 0 },
    d.choices().map((c) => ({
      label: c.label,
      checked: c.value === cur,
      action: async () => {
        try {
          acceptSettings(await d.apply(c.value));
          if (d.after) await d.after();
          refreshSettingDropdowns();
        } catch (err) {
          setStatus(t('status.settingFailed', { error: err.message }), true);
        }
      },
    })),
    el(d.id)
  );
  el('ctx-menu').dataset.dropdown = d.id;
}

function refreshSettingDropdowns() {
  for (const d of SETTING_DROPDOWNS) {
    const btn = el(d.id);
    if (btn) setText(btn, settingDropdownLabel(d));
  }
}

function bindSettingDropdowns() {
  for (const d of SETTING_DROPDOWNS) {
    el(d.id).onclick = () => toggleSettingDropdown(d);
  }
  refreshSettingDropdowns();
}

function openSettings() {
  settingsReturnFocus = document.activeElement;
  themeEditors.forEach(e => e.dispose());
  themeEditors = [mountThemeEditor('appearance-editor', 'appearance'), mountThemeEditor('reading-editor', 'reading')];
  el('set-mark-read').checked = state.settings.mark_read_on_navigate;
  refreshSettingDropdowns();
  prefetchFontFamilies();
  const proxy = state.settings.proxy || { mode: 'environment', url: '', no_proxy: '' };
  el('set-proxy-mode').value = proxy.mode;
  el('set-proxy-url').value = proxy.url;
  el('set-proxy-bypass').value = proxy.no_proxy;
  el('set-proxy-url').disabled = el('set-proxy-bypass').disabled = proxy.mode !== 'custom';
  el('set-refresh-on-start').checked = !!state.settings.refresh_on_start;
  el('set-notify-new-articles').checked = !!state.settings.notify_new_articles;
  el('set-rsshub-mirror').value = state.settings.rsshub_mirror || '';
  const dbPath = state.db ? state.db.dbPath : '';
  el('settings-db-path').textContent = dbPath;
  el('settings-db-path').title = dbPath;
  el('general-db-path').textContent = dbPath;
  fillAiForm();
  fillMcpForm();
  showPane(currentPane);
  el('settings-overlay').classList.remove('hidden');
  el('tab-' + currentPane).focus();
}

/**
 * 自绘标题栏三键（窗口无系统装饰）的最小绑定集。
 *
 * 这组绑定**必须在初始化失败路径也生效**：`loadAll()` 失败时 boot 会 return，其后的
 * 全部绑定（刷新/设置/列表/全局快捷键）都被跳过——若三键也在那时才绑，用户看到的就是
 * 一个「显示了错误状态、但没有任何可用出口」的死窗：窗口无系统装饰，只能去系统级杀进程
 * （审计 P1-1）。所以它单独成函数、放在 try 之外调用，与初始化成败无关。
 */
function bindWindowControls() {
  el('btn-win-min').onclick = () => invoke('window_minimize').catch(() => {});
  el('btn-win-max').onclick = () => invoke('window_toggle_maximize').catch(() => {});
  el('btn-win-close').onclick = () => invoke('window_close').catch((e) => {
    setStatus(t('status.settingFailed', { error: e.message }), true);
  });
}

async function boot() {
  // 点击右键菜单以外的区域时关闭菜单（菜单内部点击不受影响）。
  // 下拉触发按钮同样不算「外面」：否则开菜单的那一次点击冒泡到这里就把它关掉了
  // （菜单在同一事件里被创建又被删除，表现成「下拉点了没反应」——语言/主题/刷新
  // 间隔/字体四个自绘下拉全中招）；「再点同一个下拉 = 关闭」由 toggleSettingDropdown 管。
  document.addEventListener('click', (e) => {
    if (el('ctx-menu') && !e.target.closest('.ctx-menu') && !e.target.closest('.setting-dropdown')) {
      closeContextMenu();
    }
  });
  applyStaticI18n();
  // 库不兼容（旧开发库 / 外来 sqlite 文件）：只渲染拒绝面板与「导出 OPML」安全出口，
  // 不加载其余界面。文案由 applyStaticI18n() 按 data-i18n 填好（zh-CN / en 双语齐备）。
  try {
    const startup = await window.__TAURI__.core.invoke('startup_status');
    if (startup && startup.blocked) {
      const overlay = el('startup-refusal');
      overlay.hidden = false;
      el('startup-refusal-path').textContent = startup.db_path || '';
      el('startup-refusal-export').addEventListener('click', async () => {
        try {
          const saved = await window.__TAURI__.core.invoke('export_legacy_opml');
          if (saved) {
            el('startup-refusal-status').textContent = t('startup.refused.exported') + saved;
          }
        } catch (error) {
          el('startup-refusal-status').textContent = String(error);
        }
      });
      el('startup-refusal-quit').addEventListener('click', () => {
        window.__TAURI__.core.invoke('exit_app');
      });
      log('startup refusal overlay shown');
      // 拒绝态下左侧的「窗口自隐」还没被解除（正常路径在下方 show_main_window 处解除），
      // 不提前显示会让用户先面对一个约 5 秒的空白窗。
      window.__TAURI__.core.invoke('show_main_window').catch(() => {});
      return;
    }
  } catch (error) {
    log('startup_status unavailable: ' + error);
  }
  const i18n = i18nSelfTest();
  log(
    i18n.ok
      ? `i18n selftest ok (keys=${i18n.keys})`
      : `i18n selftest FAILED: ${i18n.problems.join('; ')}`
  );
  selfTestSanitizer();
  selfTestMenuRender();
  selfTestSubmenuPlacement();
  selfTestShortcutKeys();
  // 最小绑定集先绑、且无条件执行：下面的 catch 会 return，跳过其后的全部绑定
  bindWindowControls();
  try {
    await startThemeSync();
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
  // 列表头排序入口：图标按钮 → 锚定菜单（三档 + 隐藏已读）。
  // 必须 stopPropagation：全局 click 处理器会关掉「点击目标不在菜单里」的 ctx-menu
  // （为右键菜单与设置下拉而设），而本菜单正是在 click 处理器里打开的——不阻断
  // 冒泡的话菜单会闪一下就没（实测 2026-09-22，Xvfb 点击序列抓出来的）。
  el('btn-list-sort').onclick = (ev) => {
    ev.stopPropagation();
    openListSortMenu(ev, el('btn-list-sort'));
  };
  // 常驻「只看未读」开关与快捷键 U 共用同一个动作（见 toggleUnreadOnly）
  el('btn-unread-only').onclick = () => {
    toggleUnreadOnly().catch((err) => setStatus(err.message, true));
  };
  initRefreshEvents();
  initSidebarEvents();
  initTagEvents();
  initTagPickerEvents();
  initTagSectionEvents();
  // 色板自检放在主题应用之后：对比度算的是**当前主题**的 --bg-panel
  selfTestTagPalette();
  // 侧栏标签区行的实时矩形（无人值守拖拽定位/截图核对用）
  log(`tags:rows=${(state.sidebarTags || []).length} collapsed=${state.tagsCollapsed ? 1 : 0} rects=${tagSidebarRects()}`);
  el('btn-add').onclick = () => {
    const row = el('add-row');
    row.classList.toggle('hidden');
    if (!row.classList.contains('hidden')) el('add-url').focus();
  };
  el('btn-settings').onclick = openSettings;
  // 订阅源编辑弹窗：取消/保存 + 两个自绘下拉 + 回车即存（与其它弹窗的键盘习惯一致）
  el('feed-edit-cancel').onclick = () => closeFeedEditDialog();
  el('feed-edit-save').onclick = () => saveFeedEdit();
  el('feed-edit-name').onkeydown = (ev) => {
    if (ev.key === 'Enter') {
      ev.preventDefault();
      saveFeedEdit();
    }
  };
  el('feed-edit-folder').onclick = () => toggleFeedEditDropdown('folder');
  el('feed-edit-interval').onclick = () => toggleFeedEditDropdown('interval');
  // Esc 关弹窗（与确认框同一口径；下拉菜单同时被全局 Esc 处理器关掉是无害的，
  // 两者都是「取消」语义）
  document.addEventListener('keydown', (ev) => {
    if (ev.key === 'Escape' && !el('feed-edit-overlay').classList.contains('hidden')) {
      closeFeedEditDialog();
    }
  });
  el('ai-save').onclick = async () => {
    const key = el('ai-key').value;
    // 空值不传：后端不动这个设置（保持原值），非法值后端 clamp
    const maxTokensRaw = parseInt(el('ai-max-tokens').value, 10);
    try {
      state.ai = await invoke('save_ai_settings', {
        provider: el('ai-provider').value,
        model: el('ai-model').value,
        baseUrl: el('ai-base-url').value,
        translateTarget: el('ai-target').value,
        // 留空表示「不修改 key」，要清除得点专门的按钮
        apiKey: key.trim() === '' ? null : key,
        maxOutputTokens: Number.isFinite(maxTokensRaw) ? maxTokensRaw : null,
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
    const maxTokensRaw = parseInt(el('ai-max-tokens').value, 10);
    try {
      state.ai = await invoke('save_ai_settings', {
        provider: el('ai-provider').value,
        model: el('ai-model').value,
        baseUrl: el('ai-base-url').value,
        translateTarget: el('ai-target').value,
        apiKey: '',
        maxOutputTokens: Number.isFinite(maxTokensRaw) ? maxTokensRaw : null,
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

  el('settings-close').onclick = closeSettings;
  el('aa-close').onclick = () => el('aa-dialog').close();
  el('keyboard-help-close').onclick = () => el('keyboard-help').close();
  el('keyboard-help').addEventListener('keydown', e => e.stopPropagation());
  el('aa-dialog').addEventListener('close', () => { aaEditor?.dispose(); aaEditor = null; });
  el('aa-dialog').addEventListener('keydown', e => e.stopPropagation());

  el('settings-overlay').addEventListener('keydown', e => {
    if (e.key === 'Escape' && !el('ctx-menu')) { e.preventDefault(); e.stopPropagation(); closeSettings(); return; }
    if (e.key === 'Tab') {
      const focusable = [...el('settings-overlay').querySelectorAll('button, input, select, textarea, [tabindex="0"]')].filter(n => !n.disabled && n.tabIndex >= 0 && n.getClientRects().length);
      const first = focusable[0], last = focusable.at(-1);
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last?.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
    }
    if (!e.target.closest('#settings-nav-list')) return;
    const tabs = [...el('settings-nav-list').querySelectorAll('[data-pane]')];
    let i = tabs.indexOf(document.activeElement);
    if (e.key === 'ArrowDown') i = (i + 1) % tabs.length;
    else if (e.key === 'ArrowUp') i = (i + tabs.length - 1) % tabs.length;
    else if (e.key === 'Home') i = 0;
    else if (e.key === 'End') i = tabs.length - 1;
    else return;
    e.preventDefault(); currentPane = tabs[i].dataset.pane; showPane(currentPane); tabs[i].focus();
  });

  // 左栏分类切换（事件委派：以后加分类不用改这里）
  el('settings-nav-list').addEventListener('click', (e) => {
    const li = e.target.closest('button[data-pane]');
    if (!li) return;
    currentPane = li.dataset.pane;
    showPane(currentPane);
    log(`settings pane=${currentPane}`);
  });

  // 自绘标题栏三键的绑定已在 boot 开头的 bindWindowControls() 里完成（无条件执行）
  el('btn-new-folder').onclick = () => createFolder();
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
      log(`rsshub normalize: migrated=${out.migrated} skipped=${out.skipped} errors=${out.errors.length}`);
      await refreshCounts();
    } catch (err) {
      el('rsshub-status').textContent = err.message;
    }
  };
  el('settings-overlay').addEventListener('click', (e) => {
    // 点击遮罩区域关闭（点对话框内部不关）
    if (e.target === el('settings-overlay')) closeSettings();
  });
  el('set-mark-read').addEventListener('change', async (e) => {
    try {
      acceptSettings(await invoke('set_mark_read_on_navigate', { enabled: e.target.checked }));
      setStatus(t(e.target.checked ? 'status.markReadOn' : 'status.markReadOff'));
      log(`setting mark_read_on_navigate=${e.target.checked}`);
    } catch (err) {
      setStatus(t('status.settingFailed', { error: err.message }), true);
    }
  });
  el('set-proxy-mode').addEventListener('change', () => {
    el('set-proxy-url').disabled = el('set-proxy-bypass').disabled = el('set-proxy-mode').value !== 'custom';
  });
  el('set-proxy-save').addEventListener('click', async () => {
    const button = el('set-proxy-save');
    button.disabled = true;
    try {
      const mode = el('set-proxy-mode').value;
      const config = { mode, url: mode === 'custom' ? el('set-proxy-url').value.trim() : '',
        no_proxy: mode === 'custom' ? el('set-proxy-bypass').value.trim() : '' };
      acceptSettings(await invoke('set_proxy_config', { config }));
      el('set-proxy-status').textContent = t('settings.proxy.saved');
    } catch (error) {
      el('set-proxy-status').textContent = t(error.message.includes('proxy_credentials_not_supported')
        ? 'settings.proxy.credentials' : 'settings.proxy.invalid');
    } finally { button.disabled = false; }
  });
  el('set-refresh-on-start').addEventListener('change', async (e) => {
    try {
      acceptSettings(await invoke('set_refresh_on_start', { enabled: e.target.checked }));
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
  el('set-notify-new-articles').addEventListener('change', async (e) => {
    try {
      acceptSettings(await invoke('set_notify_new_articles', { enabled: e.target.checked }));
      e.target.checked = state.settings.notify_new_articles;
      log(`notifyNewArticles=${state.settings.notify_new_articles}`);
    } catch (err) {
      setStatus(t('status.settingFailed', { error: err.message }), true);
      log(`set_notify_new_articles failed: ${err.message}`);
    }
  });
  el('btn-list-bulk').onclick = e => {
    e.stopPropagation();
    openContextMenu(e, [
      { label: t('settings.markAllRead'), action: () => markAll(true) },
      { label: t('settings.markAllUnread'), action: () => markAll(false) },
    ], el('btn-list-bulk'));
  };

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
  // 备份 / 恢复：两个动作都在 Rust 侧弹原生对话框（目录 / 文件 / 覆盖确认），
  // 前端只负责提示结果；恢复是「暂存 + 重启生效」，所以文案里必须带重启提醒。
  el('act-backup-db').onclick = async () => {
    try {
      const path = await invoke('backup_db');
      setStatus(path ? t('status.backupDone', { path }) : t('status.backupCancelled'));
      log(`backup_db ${path ?? 'cancelled'}`);
    } catch (err) {
      setStatus(t('status.backupFailed', { error: err.message }), true);
      log(`backup_db failed: ${err.message}`);
    }
  };

  el('act-restore-db').onclick = async () => {
    try {
      const path = await invoke('restore_db');
      setStatus(path ? t('status.restoreStaged') : t('status.restoreCancelled'));
      log(`restore_db ${path ?? 'cancelled'}`);
    } catch (err) {
      setStatus(t('status.restoreFailed', { error: err.message }), true);
      log(`restore_db failed: ${err.message}`);
    }
  };

  // 关于页「打开日志目录」：Rust 侧先确保目录存在，再交给系统文件管理器（不内嵌查看器）。
  // 失败（没装 xdg-open / 启动器打不开）只把可读错误放进状态栏，不弹窗、不影响其它功能。
  el('act-open-logs').onclick = async () => {
    try {
      await invoke('open_logs_dir');
      setStatus(t('status.logsDirOpened'));
      log('open_logs_dir ok');
    } catch (err) {
      setStatus(t('status.logsDirFailed', { error: err.message }), true);
      log(`open_logs_dir failed: ${err.message}`);
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

  // ---- 写能力：两个开关 + 写 token 生成/轮换/销毁（命令返回整张视图，重填表单）
  const runMcpWriteCommand = async (command, noteKey, logLine) => {
    try {
      state.mcp = await invoke(command);
      fillMcpForm();
      if (noteKey) setStatus(t(noteKey));
      log(logLine(state.mcp));
    } catch (err) {
      el('mcp-write-token').textContent = t('settings.mcp.failed', { error: err.message });
      log(`mcp write ${command} failed: ${err.message}`);
    }
  };
  // 写能力的四态在日志里如实落盘（agent 排障要看“有没有开通写能力”）
  const writeState = (mcp) =>
    `writeEnabled=${mcp.write_enabled} dangerous=${mcp.dangerous_enabled} writeToken=${mcp.write_token ? 'set' : 'none'}`;

  el('set-mcp-write-enabled').addEventListener('change', async (e) => {
    try {
      state.mcp = await invoke('set_mcp_write_enabled', { enabled: e.target.checked });
      fillMcpForm();
      log(`mcp write_enabled=${e.target.checked} ${writeState(state.mcp)}`);
    } catch (err) {
      e.target.checked = !e.target.checked;
      el('mcp-write-token').textContent = t('settings.mcp.failed', { error: err.message });
      log(`mcp write_enabled failed: ${err.message}`);
    }
  });

  el('set-mcp-dangerous-enabled').addEventListener('change', async (e) => {
    try {
      state.mcp = await invoke('set_mcp_dangerous_enabled', { enabled: e.target.checked });
      fillMcpForm();
      log(`mcp dangerous_enabled=${e.target.checked} ${writeState(state.mcp)}`);
    } catch (err) {
      e.target.checked = !e.target.checked;
      el('mcp-write-token').textContent = t('settings.mcp.failed', { error: err.message });
      log(`mcp dangerous_enabled failed: ${err.message}`);
    }
  });

  el('mcp-write-generate').onclick = () =>
    runMcpWriteCommand(
      'generate_mcp_write_token',
      'settings.mcp.writeGenerated',
      (mcp) => `mcp write token generated ${writeState(mcp)}`
    );

  el('mcp-write-rotate').onclick = () =>
    runMcpWriteCommand(
      'rotate_mcp_write_token',
      'settings.mcp.writeRotated',
      (mcp) => `mcp write token rotated ${writeState(mcp)}`
    );

  el('mcp-write-clear').onclick = () =>
    runMcpWriteCommand(
      'clear_mcp_write_token',
      'settings.mcp.writeCleared',
      (mcp) => `mcp write token cleared ${writeState(mcp)}`
    );

  el('mcp-write-copy').onclick = async () => {
    try {
      // 写 token 单独复制（客户端配置片段里只有读 token）
      await invoke('clip_write', { text: state.mcp.write_token || '' });
      setStatus(t('settings.mcp.writeCopied'));
      log('mcp write token copied');
    } catch (err) {
      setStatus(t('settings.mcp.copyFailed', { error: err.message }), true);
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

  document.addEventListener('keydown', onGlobalKeydown);
}

/**
 * 全局快捷键分发（唯一的键盘入口）。
 *
 * 键位表：j/↓ k/↑ 移动 · Enter 打开 · u 未读 · s 星标 · l 稍后读 · t 标签选择器 ·
 * r 刷新 · g 首行 · G 末行；`/` 聚焦搜索、Esc 关菜单/清搜索/退出标签视图——
 * 后两个在 switch 之前单独处理（Esc 要先让位给菜单与覆盖层）。
 * 自检见 selfTestShortcutKeys()（键位从本函数源码抽取，不另维护清单）。
 */
function onGlobalKeydown(e) {
  const inField = ['INPUT', 'TEXTAREA', 'SELECT'].includes(document.activeElement?.tagName);
  if (el('aa-dialog').open || el('keyboard-help').open) return;
  if (!el('settings-overlay').classList.contains('hidden') && e.key !== 'Escape') return;
  // 确认框自己处理 Esc/Enter，其它全局快捷键先让位
  if (!el('ai-confirm-overlay').classList.contains('hidden')) return;
  if (e.key === '/' && !inField) {
    e.preventDefault();
    el('search').focus();
    return;
  }
  if (e.key === 'Escape') {
    // 菜单树优先，且只关菜单：开着菜单按 Esc 不该顺手把搜索态/设置面板一起清掉。
    // （注意：本判断原先排在下面那个 return 之后，永远走不到——菜单 Esc 关不掉，
    // 本次一并修好；这条也是本任务「Esc 关闭整棵菜单树」的验收点。）
    if (el('ctx-menu')) {
      closeContextMenu();
      return;
    }
    el('settings-overlay').classList.add('hidden');
    el('search').value = '';
    el('search').blur();
    el('add-row').classList.add('hidden');
    // 搜索态与标签视图都是「临时筛选视图」，Esc 退回未读（默认视图）
    if (state.view.kind === 'search' || state.view.kind === 'tag') setView({ kind: 'unread' });
    return;
  }
  if (inField || e.ctrlKey || e.metaKey || e.altKey) return;
  // 选择器开着时全局键一律不生效（它自己的捕获阶段监听已消化 ↑↓/Enter/Esc；
  // 这里兜底的是「焦点被鼠标点到列表行上」之后按 j/k/u/s/l/t 的情况）
  if (tagPickerOpen()) return;
  // 设置面板开着时，导航类快捷键同样让位（否则 j/k 会在面板背后换文章）
  if (!el('settings-overlay').classList.contains('hidden')) return;
  // 编辑弹窗同理：背后的列表/阅读区不该被快捷键推动（Esc 在上面已处理）
  if (!el('feed-edit-overlay').classList.contains('hidden')) return;

  switch (e.key) {
      case '?': e.preventDefault(); el('keyboard-help').showModal(); break;
      case 'j': case 'ArrowDown': e.preventDefault(); move(1); break;
      case 'k': case 'ArrowUp': e.preventDefault(); move(-1); break;
      case 'Enter': e.preventDefault(); if (state.selectedId) openEntry(state.selectedId, { markRead: true }); break;
      case 'u': e.preventDefault(); toggleRead().catch((err) => setStatus(err.message, true)); break;
      case 's': e.preventDefault(); toggleStar().catch((err) => setStatus(err.message, true)); break;
      case 'l': e.preventDefault(); toggleReadLater().catch((err) => setStatus(err.message, true)); break;
      // 与 u/s/l 同族：给「当前这篇」打开选择器（焦点落在输入框）。
      // 列表态（还没选行/正文区是占位）时退到首行——选择器头部会写明是给哪篇打标，
      // 且只有 Enter/点击才会写入，所以不是「静默改了用户没选的文章」。
      case 't': e.preventDefault(); openTagPicker(state.readerEntry?.id ?? state.selectedId ?? state.entries[0]?.id).catch((err) => setStatus(err.message, true)); break;
      // 大写 U = 「只看未读」列表总开关（小写 u 是切换当前这篇的已读态，两者语义正交）
      case 'U': e.preventDefault(); toggleUnreadOnly().catch((err) => setStatus(err.message, true)); break;
      // 大写 A = 「当前视图全部标为已读」（与菜单里的同名动作同一个入口 markAll）。
      // 用小写 a 容易误触（它是高频字母），且「全部已读」是不可逆的批量操作，
      // 跟 U / G 一样用大写。markAll 内部按当前视图 scope 走，无额外确认弹窗。
      case 'A': e.preventDefault(); markAll(true).catch((err) => setStatus(err.message, true)); break;
      case 'r': e.preventDefault(); doRefresh(); break;
      case 'g': e.preventDefault(); jump(false); break;
      case 'G': e.preventDefault(); jump(true); break;
      default: break;
  }
}

window.addEventListener('DOMContentLoaded', boot);

})();
