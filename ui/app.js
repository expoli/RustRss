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

// ---------------------------------------------------------------- 字体配置

/// 正文字号 / 行高的区间、步长与默认值：与 Rust 侧 clamp 区间同源
/// （HTML 的 min/max/step 只是静态兜底，真正的判据在这里 + Rust）。
const FONT_SIZE = { min: 13, max: 18, step: 1, fallback: 14 };
const FONT_LINE = { min: 1.5, max: 1.8, step: 0.05, fallback: 1.55 };

/// 数值兜底：非有限值回默认，越界夹回区间（库里被写坏也不让排版崩）
function clampNumber(value, { min, max, fallback }) {
  const n = Number(value);
  if (!Number.isFinite(n)) return fallback;
  return Math.min(max, Math.max(min, n));
}

/// 数值显示口径：字号取整、行高两位小数（与落库口径一致，标签不跳字）
const fontSizeText = (v) => String(clampNumber(v, FONT_SIZE));
const fontLineText = (v) => clampNumber(v, FONT_LINE).toFixed(2);

/// 单个字体族名 → CSS `font-family` 值。
/// 只有「简单标识符」能裸写；含空格 / CJK / 逗号 / 引号的一律加引号并转义——
/// 否则 CSS 会把 "Noto Sans CJK SC" 拆成三个家族名（自定义属性不会被当字符串）。
/// 通用族（sans-serif 等）走裸写分支：加引号就变成名叫 "sans-serif" 的具体家族了。
const CSS_IDENT_FAMILY = /^[A-Za-z][A-Za-z0-9_-]*$/;
function cssFamily(name) {
  const trimmed = String(name || '').trim();
  if (!trimmed) return '';
  if (CSS_IDENT_FAMILY.test(trimmed)) return trimmed;
  return `"${trimmed.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
}

/// 已写入内联样式的变量值：同值短路，重复应用（启动 / 切换 / 滑块松手）零 DOM 写入
/// （性能红线 #7）。值 `undefined` 表示从未写过，`''` 表示已清除。
const appliedFontVars = {};

/// 写 / 清一个 CSS 变量：空值 = 清除 → 回落到 :root 的内置字体栈（「跟随系统」）
function setFontVar(name, value) {
  if (appliedFontVars[name] === value) return;
  appliedFontVars[name] = value;
  const style = document.documentElement.style;
  if (value) style.setProperty(name, value);
  else style.removeProperty(name);
}

/// 拖动预览的覆盖值：拖动期间（input 事件）累积在这里，落库（change）后清掉。
/// 这样「先拖字号再拖行高」不会因为合并了没入库的 state 而把前一项预览抖回旧值。
const fontPreview = {};

/// 应用字体设置：`state.settings`（Rust 侧权威值）+ 正在拖动中的预览覆盖。
/// **只写 CSS 变量，不动滑块位置**——预览若顺手把控件按已保存值重画，
/// 另一个滑块会被拽回它的旧值（两个滑块互相干扰），控件同步单独走 paintFontControls。
function applyFontConfig() {
  const s = { ...state.settings, ...fontPreview };
  setFontVar('--font-ui', cssFamily(s.font_ui));
  setFontVar('--font-read', cssFamily(s.font_read));
  setFontVar('--font-mono', cssFamily(s.font_mono));
  setFontVar('--font-read-size', `${fontSizeText(s.font_read_size)}px`);
  setFontVar('--font-read-line', fontLineText(s.font_read_line));
}

/// 滑块位置与数值标签对齐给定设置。只在「拿到完整权威值」时调：
/// 启动 / 打开设置页 / 保存成功 / 保存失败回滚。同值短路（拖动一秒几十个事件）。
function paintFontControls(s) {
  const size = clampNumber(s.font_read_size, FONT_SIZE);
  const line = clampNumber(s.font_read_line, FONT_LINE);
  const sizeInput = el('set-font-size');
  const lineInput = el('set-font-line');
  // 位置只在真的不同时才写：拖动中不能跟用户的手抢滑块
  if (sizeInput && Number(sizeInput.value) !== size) sizeInput.value = String(size);
  if (lineInput && Number(lineInput.value) !== line) lineInput.value = String(line);
  paintFontValueLabel('font_read_size', size);
  paintFontValueLabel('font_read_line', line);
}

/// 只刷新某一个滑块的数值标签（拖动预览路径用：不碰位置，也不碰另一个滑块）
function paintFontValueLabel(field, value) {
  const label = el(field === 'font_read_size' ? 'set-font-size-value' : 'set-font-line-value');
  if (!label) return;
  setText(label, field === 'font_read_size' ? `${fontSizeText(value)}px` : fontLineText(value));
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
  // 会话内已读的条目 id：未读视图里读过一行就从列表删掉，后台刷新 prepend 时不能再把它
  // 插回来（服务端 unread 过滤在竞态下会漏——刷新的查询可能先于 set_read 提交）。
  readSessionIds: new Set(),
  view: { kind: 'unread' },
  feedId: null,
  selectedId: null,
  // 正文区当前展示条目所属的源 id：改源名后只更新该源的元信息（正文不重渲染）
  readerFeedId: null,
  query: '',
  // 权威值在 Rust 侧（get_ui_settings），这里只是启动前的占位
  settings: { mark_read_on_navigate: true },
  // 系统字体族（设置页打开时预取一次并缓存；空 = 未取到/非 Linux → 只剩「跟随系统」）
  fontFamilies: [],
  fontFamiliesLoaded: false,
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
  const tooltip = failed
    ? t('sidebar.feedTooltipFailed', {
        status: f.last_status,
        error: f.last_error || '',
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

/// 单行构造：首屏与续页共用（续页只 append 新行，已有的行一个都不重建）。
function buildEntryRow(e) {
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
  return li;
}

/// 列表计数口径：未读视图只数未读行（灰显的已读行还在列表里但不算数），
/// 其余视图数全部行。
function listCountN() {
  return state.view.kind === 'unread'
    ? state.entries.filter((e) => !e.read).length
    : state.entries.length;
}

function renderList() {
  const __t0 = performance.now();
  el('list-title').textContent = viewTitle();
  el('list-count').textContent = state.entries.length ? t('list.count', { n: listCountN() }) : '';

  const list = el('entries');
  list.innerHTML = '';
  if (!state.entries.length) {
    const li = document.createElement('li');
    li.className = 'dim';
    li.style.cursor = 'default';
    li.textContent = state.view.kind === 'unread' ? t('list.emptyUnread') : t('list.empty');
    list.appendChild(li);
    installSentinel();
    return;
  }

  for (const e of state.entries) list.appendChild(buildEntryRow(e));
  installSentinel();
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

// ------------------------------------------- 列表续页（尾部哨兵 + IntersectionObserver）

/// 尾部哨兵：滚到接近底部（rootMargin 600px）就自动续下一页。
/// 每次 renderList/续页之后都换一个新哨兵节点再 observe——IntersectionObserver 只在
/// 「相交状态变化」时回调，哨兵一直留在可视区内（这批行不够填满一屏、或列表被读空
/// 变短）时不会再有通知、续页就卡住了；重新 observe 必然先给一次初始通知，正好把
/// 「还没填满就接着拉」接上。
let sentinel = null;
let listObserver = null;

function installSentinel() {
  const list = el('entries');
  if (listObserver && sentinel) listObserver.unobserve(sentinel);
  if (sentinel) sentinel.remove();
  sentinel = null;
  // 没有下一页（已到末尾、搜索本版不分页、续页刚失败）就不留哨兵
  if (paging.exhausted || paging.error || state.view.kind === 'search') return;
  if (!listObserver) {
    listObserver = new IntersectionObserver(onSentinel, { root: list, rootMargin: '600px' });
  }
  sentinel = document.createElement('li');
  sentinel.className = 'load-sentinel dim';
  sentinel.style.cursor = 'default';
  list.appendChild(sentinel);
  listObserver.observe(sentinel);
}

function onSentinel(records) {
  if (records.some((r) => r.isIntersecting)) loadMore();
}

function renderReaderEmpty() {
  state.readerFeedId = null;
  el('reader').innerHTML = `<div class="reader-empty">
      <p>${t('reader.empty')}</p>
      <p class="dim">${t('reader.shortcuts')}</p>
    </div>`;
}

function renderReader(entry) {
  state.readerFeedId = entry.feed_id;
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
    <div class="article">${body}</div>`);

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
  // 字体同理：先把 CSS 变量设好再渲染（跨会话保持的字体在首帧就生效）
  applyFontConfig();
  bindFontControls();
  bindSettingDropdowns();
  applyStaticI18n();
  renderSidebar();
  await loadEntries({ reader });
  log(
    `loaded feeds=${sidebar.db.feeds} entries=${sidebar.db.entries} unread=${sidebar.db.unread} starred=${sidebar.db.starred} markReadOnNavigate=${settings.mark_read_on_navigate} refreshInterval=${settings.refresh_interval_minutes} refreshOnStart=${settings.refresh_on_start} notifyNewArticles=${settings.notify_new_articles} fonts ui=${settings.font_ui || 'default'} read=${settings.font_read || 'follow-ui'} mono=${settings.font_mono || 'default'} size=${fontSizeText(settings.font_read_size)}px line=${fontLineText(settings.font_read_line)} ai=${ai.provider}${ai.model ? '/' + ai.model : '（未配模型）'} hasKey=${ai.has_key} mcp=${mcp.running ? mcp.url : 'off'}${reader ? '' : ' silent（正文未重渲染）'}`
  );
}

/// 每批条数：与后端 list_entries 的默认值一致，也是「有没有下一页」的判据。
const PAGE_SIZE = 200;

/// 列表分页状态（keyset 复合游标）。
/// cursor 记的是「已取到的最后一行」而不是「当前列表最后一行」：未读视图里读完一篇
/// 会把它从列表里移除，若拿剩下的末行当游标，读空一整批之后就再也取不到后面的未读
/// 条目——游标是结果流里的位置，不随某行被移出列表而后退。
const paging = { cursor: null, exhausted: false, loading: false, error: false };

/// 当前视图对应的 `list_entries` 参数（`cursor: null` = 取首页）。
/// 首页与续页必须用同一套筛选，抽出来避免后台刷新的 prepend 比对另抄一份漂移。
function listArgs(cursor) {
  const kind = state.view.kind;
  return {
    feedId: kind === 'feed' ? state.feedId : null,
    unreadOnly: kind === 'unread',
    starredOnly: kind === 'starred',
    readLaterOnly: kind === 'later',
    limit: PAGE_SIZE,
    // 游标 = 上一页末行的 (sortkey, id)；sortkey 由后端直出，前端不按
    // published_at/fetched_at 自己算（前端也拿不到 fetched_at）
    cursorSortkey: cursor ? cursor.sortkey : null,
    cursorId: cursor ? cursor.id : null,
  };
}

/// 取一页并记下游标与「有没有下一页」。判据是「返回不足一批」：满批也可能是最后一页，
/// 多请求一次空页的代价可以接受，换来的是不必猜。
async function loadPage(cursor) {
  const rows = await invoke('list_entries', listArgs(cursor));
  const last = rows[rows.length - 1];
  if (last) paging.cursor = { sortkey: last.sortkey, id: last.id };
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
  const first = state.entries[0];
  const rows = await invoke('list_entries', listArgs(null));
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
  el('list-count').textContent = t('list.count', { n: listCountN() });
  log(
    `refresh:done prepend rows=${fresh.length} ids=${fresh.map((e) => e.id).join(',')} sessionReadSkipped=${skipped} atTop=${atTop} listScrollTop=${scrollBefore}→${list.scrollTop} height=${heightBefore}→${list.scrollHeight} top=${visibleBefore}→${topVisibleRowId(list)} head=${list.firstChild?.dataset.id ?? 'none'} children=${list.children.length} total=${state.entries.length}`
  );
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

  // 搜索本版仍是一次性 200（PRD R3）：不装哨兵、不参与续页
  if (kind === 'search') {
    paging.cursor = null;
    paging.exhausted = true;
    paging.error = false;
    state.entries = state.query ? await invoke('search', { query: state.query, limit: PAGE_SIZE }) : [];
    renderList();
    if (reader) renderSelectedEntry();
    log(`view=${kind} count=${state.entries.length} exhausted=true`);
    return;
  }

  if (!reset && paging.cursor) {
    const rows = await loadPage(paging.cursor);
    // 运行时自证：续页不该重放已加载的行（keyset 游标保证不重不漏）。dup 一旦非 0
    // 就是游标语义坏了，日志里必须看得见，而不是等用户发现列表里有重复行。
    const seen = new Set(state.entries.map((e) => e.id));
    const dup = rows.filter((e) => seen.has(e.id));
    const list = el('entries');
    state.entries = state.entries.concat(rows);
    for (const e of rows) list.appendChild(buildEntryRow(e));
    el('list-count').textContent = t('list.count', { n: listCountN() });
    installSentinel();
    log(
      `append rows=${rows.length} total=${state.entries.length} dup=${dup.length} exhausted=${paging.exhausted}`
    );
    return;
  }

  paging.cursor = null;
  paging.exhausted = false;
  paging.error = false;
  // 列表要整体重建（换视图/换筛选/手动刷新）：会话已读集合对应的「已删除行」没了，清空
  state.readSessionIds.clear();
  state.entries = await loadPage(null);
  // 静默刷新（reader=false）不动 selectedId：正文区一个 DOM 都不动，选中态也
  // 保持——否则重指到首行后，操作按钮（标已读/星标/稍后读）会作用于用户没在看的
  // 文章（实测 2026-09-22：后台刷新把 selected 挪到 9320 而正文还是 9135）。
  // 静默刷新（reader=false）不动 selectedId；视图切换（reader=true）也不再回退
  // 选中首行——保持「点击才算已读」，阅读区显示占位（见 renderSelectedEntry）。
  renderList();
  // 静默模式到此为止：正文区一个 DOM 都不动
  if (!reader) return;
  await renderSelectedEntry();
  log(
    `view=${kind}${state.feedId ? '#' + state.feedId : ''} count=${state.entries.length} exhausted=${paging.exhausted}`
  );
}

/// 续页（哨兵触发）。防重入：同一时刻只允许一个请求在飞——滚动抖动会让哨兵连续触发，
/// 用同一个游标并发请求会把同一批行 append 两遍（列表出现重复行）。
async function loadMore() {
  if (paging.loading || paging.exhausted || paging.error || !paging.cursor) return;
  const kind = state.view.kind;
  paging.loading = true;
  if (sentinel) sentinel.textContent = t('list.loadingMore');
  try {
    await loadEntries({ reader: false, reset: false });
  } catch (err) {
    // 失败即停：留着哨兵会立刻重试（新节点必然收到初始通知），把后端和日志打满；
    // 恢复走换视图/换筛选（reset 路径）或手动刷新。
    paging.error = true;
    installSentinel();
    setStatus(t('status.loadMoreFailed', { error: err.message }), true);
    log(`loadMore failed view=${kind}: ${err.message}`);
  } finally {
    paging.loading = false;
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
  if (state.view.kind === 'unread') {
    el('list-count').textContent = t('list.count', { n: listCountN() });
  }
  // renderReader 用的是 set_read 前取的 entry：按钮文案会滞后一拍（已读却写着
  // 「标为已读」）。只改这一个按钮的文本，不重渲染整个阅读区。
  const readBtn = el('act-read');
  if (readBtn) readBtn.textContent = t('reader.markUnread');
  // 计数刷新节流：连续快速阅读时合并为一次全量刷新（600ms 去抖）
  refreshCountsSoon();
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

/// 通用右键菜单：items = [{label, danger?, action} | {separator: true} | {header: true, label}]
/// - `separator` 画一条分组分隔线；`header` 是小号灰字的分组标题（不可点）。
/// - `checked` 参与勾选组：勾中项前面打 ✓，未勾中项留同宽占位（标签对齐）。
function openContextMenu(ev, items, anchor) {
  closeContextMenu();
  const menu = document.createElement('div');
  menu.id = 'ctx-menu';
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
    } else {
      btn.textContent = item.label;
    }
    btn.onclick = () => {
      closeContextMenu();
      item.action();
    };
    menu.appendChild(btn);
  }
  document.body.appendChild(menu);
  const pad = 8;
  // 锚定模式（设置页下拉）：贴着触发按钮下沿展开，宽度不小于按钮宽；
  // 事件坐标模式（右键菜单）行为不变
  if (anchor) {
    const r = anchor.getBoundingClientRect();
    menu.style.minWidth = r.width + 'px';
    menu.style.left = Math.min(r.left, window.innerWidth - menu.offsetWidth - pad) + 'px';
    menu.style.top = Math.min(r.bottom + 2, window.innerHeight - menu.offsetHeight - pad) + 'px';
  } else {
    menu.style.left = Math.min(ev.clientX, window.innerWidth - menu.offsetWidth - pad) + 'px';
    menu.style.top = Math.min(ev.clientY, window.innerHeight - menu.offsetHeight - pad) + 'px';
  }
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
  // 立即刷新置顶：原先只能双击源标题触发（可发现性差，实测用户不知道）；
  // 与移动/刷新间隔组用分隔线隔开
  items.push({ label: t('menu.refreshNow'), action: () => refreshOne(feed.id) });
  items.push({ separator: true });
  // 编辑：标题/文件夹/间隔三件套的收敛入口（与下面的快捷项同一落库路径）
  items.push({ label: t('menu.editFeed'), action: () => openFeedEditDialog(feed) });
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
  // 刷新间隔组：与「移入文件夹」同层、分隔线隔开；当前档位打勾（FeedRow 直出）
  const current =
    feed.refresh_interval_minutes == null ? null : String(feed.refresh_interval_minutes);
  // 没有「移动」项时（无分组可移动）不画分隔线——否则菜单顶部多一条没意义的线。
  if (items.length) items.push({ separator: true });
  items.push({ header: true, label: t('menu.refreshInterval') });
  for (const choice of FEED_REFRESH_CHOICES) {
    items.push({
      label: choice.label(),
      checked: current === choice.value,
      action: () => setFeedRefreshInterval(feed.id, choice.value),
    });
  }
  // 取消订阅：破坏性（条目级联删），与刷新间隔组分隔开，红色危险样式
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

function reassignFeed(feedId, folderId) {
  invoke('assign_feed_folder', { feedId, folderId })
    .then(() => refreshCounts())
    .catch((err) => setStatus(err.message, true));
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
  if (read) state.readSessionIds.add(row.id);
  else state.readSessionIds.delete(row.id);
  row.read = read;
  // 未读视图同样只灰显不删行：按钮切换后留在原文章（不未经请求地跳到下一篇），
  // 双向都生效（读→灰、取消未读→恢复），计数口径同步更新。
  const fresh = await invoke('get_entry', { id: row.id });
  if (fresh) renderReader(fresh);
  // 只改那一行的已读样式，不重建列表
  for (const li of el('entries').children) {
    if (li.dataset.id === String(row.id)) li.classList.toggle('read', read);
  }
  if (state.view.kind === 'unread') {
    el('list-count').textContent = t('list.count', { n: listCountN() });
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
    // 批量标记同样算「会话内读过」：prepend 不回插（紧随其后的 loadAll 会重建列表并
    // 清空集合，这里跟上单条路径的语义，不让两条路径对不上）
    for (const e of state.entries) {
      if (read) state.readSessionIds.add(e.id);
      else state.readSessionIds.delete(e.id);
    }
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
  el('ai-max-tokens').value = ai.max_output_tokens || 4096;
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
    apply: (v) => invoke('set_ui_locale', { locale: v }),
    after: async () => {
      setLocale(state.settings.locale || 'auto');
      applyStaticI18n();
      renderSidebar();
      renderList();
      if (state.selectedId) {
        const entry = await invoke('get_entry', { id: state.selectedId });
        if (entry) renderReader(entry);
      }
    },
  },
  {
    id: 'set-theme',
    choices: () => [
      { value: 'system', label: t('settings.themeSystem') },
      { value: 'light', label: t('settings.themeLight') },
      { value: 'dark', label: t('settings.themeDark') },
    ],
    current: () => state.settings.theme || 'system',
    apply: (v) => invoke('set_ui_theme', { theme: v }),
    after: () => applyTheme(state.settings.theme || 'system'),
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
  // 字体：choices 来自 `list_font_families`（设置页打开时预取并缓存，见 prefetchFontFamilies）。
  // 只传自己那一个参数：Rust 侧 `set_font_config` 对 `null` 的项不动库里原有值。
  {
    id: 'set-font-ui',
    choices: () => fontChoices('font_ui'),
    current: () => state.settings.font_ui || '',
    apply: (v) => invoke('set_font_config', { fontUi: v }),
    after: () => applyFontConfig(),
  },
  {
    id: 'set-font-read',
    choices: () => fontChoices('font_read'),
    current: () => state.settings.font_read || '',
    apply: (v) => invoke('set_font_config', { fontRead: v }),
    after: () => applyFontConfig(),
  },
  {
    id: 'set-font-mono',
    choices: () => fontChoices('font_mono'),
    current: () => state.settings.font_mono || '',
    apply: (v) => invoke('set_font_config', { fontMono: v }),
    after: () => applyFontConfig(),
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
];

function settingDropdownLabel(d) {
  const cur = d.current();
  return (d.choices().find((c) => c.value === cur) || d.choices()[0]).label;
}

/// 字体下拉的候选：首项「跟随系统」（value = ''），其余是系统字体族（取不到就只有首项）。
/// 当前值不在列表里时补一项：换了机器 / 卸了字体后标签要如实显示真实值，
/// 而不是静默显示「跟随系统」（那会让用户以为设置丢了）。
function fontChoices(settingKey) {
  const cur = String(state.settings[settingKey] || '').trim();
  const choices = [{ value: '', label: t('settings.fontFollowSystem') }];
  for (const name of state.fontFamilies) choices.push({ value: name, label: name });
  if (cur && !state.fontFamilies.includes(cur)) choices.push({ value: cur, label: cur });
  return choices;
}

/// 预取系统字体族：**只在第一次打开设置页时**跑（fc-list 是子进程，不进启动路径）；
/// 结果缓存在 state.fontFamilies，取到后重画三个下拉的标签。
/// 失败与空表都不是错误：字体下拉仍有「跟随系统」，只是把原因挂到 tooltip 上。
function prefetchFontFamilies() {
  if (state.fontFamiliesLoaded) {
    syncFontListHints();
    return;
  }
  state.fontFamiliesLoaded = true; // 先置位：多次打开不重复 spawn
  invoke('list_font_families')
    .then((families) => {
      state.fontFamilies = Array.isArray(families) ? families : [];
      syncFontListHints();
      refreshSettingDropdowns();
      log(
        state.fontFamilies.length
          ? `font families=${state.fontFamilies.length}`
          : 'font families=0（只有「跟随系统」可选）'
      );
    })
    .catch((e) => {
      state.fontFamilies = [];
      syncFontListHints();
      log(`list_font_families failed: ${e.message}（字体下拉只有「跟随系统」）`);
    });
}

/// 字体列表为空时把原因写到三个下拉的 tooltip：不新增可见元素，因此不会引起布局跳动
/// （性能红线 #8）。列表非空时清掉 tooltip。
function syncFontListHints() {
  if (!state.fontFamiliesLoaded) return;
  const hint = state.fontFamilies.length ? '' : t('settings.fontListEmpty');
  for (const id of ['set-font-ui', 'set-font-read', 'set-font-mono']) {
    const btn = el(id);
    if (btn) btn.title = hint;
  }
}

/// 设置字段 → `set_font_config` 的 IPC 参数名（Tauri 侧 camelCase → snake_case）
const FONT_SETTING_ARGS = { font_read_size: 'readSize', font_read_line: 'readLine' };

/// 字号 / 行高滑块：
/// - `input`（拖动中）只写 CSS 变量做实时预览，不碰库、不发 IPC；
/// - `change`（松手）才调 `set_font_config` 落库，拿返回值回刷（Rust 是权威值）。
/// 拖动一秒会打出几十个 input 事件，其中绝大部分是同一个 step 值——同值直接 return
/// （性能红线 #7 的同值短路；日志也因此每个档位只有一行，可当预览证据用）。
function bindFontControls() {
  const bind = (inputId, key, spec) => {
    const input = el(inputId);
    if (!input) return;
    let lastPreview = null;
    input.oninput = () => {
      const value = clampNumber(input.value, spec);
      if (value === lastPreview) return;
      lastPreview = value;
      fontPreview[key] = value;
      applyFontConfig();
      paintFontValueLabel(key, value);
      log(`font ${key}=${value} preview（仅 CSS 变量）`);
    };
    input.onchange = () => {
      const value = clampNumber(input.value, spec);
      invoke('set_font_config', { [FONT_SETTING_ARGS[key]]: value })
        .then((settings) => {
          state.settings = settings;
          delete fontPreview[key];
          applyFontConfig();
          paintFontControls(state.settings);
          log(`font ${key}=${value} saved`);
        })
        .catch((e) => {
          // 没写进库就把界面退回已保存值：不留下「看着生效了、重启又变回去」的偏差
          delete fontPreview[key];
          applyFontConfig();
          paintFontControls(state.settings);
          setStatus(t('status.settingFailed', { error: e.message }), true);
        });
    };
  };
  bind('set-font-size', 'font_read_size', FONT_SIZE);
  bind('set-font-line', 'font_read_line', FONT_LINE);
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
          state.settings = await d.apply(c.value);
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
    if (btn) btn.textContent = settingDropdownLabel(d);
  }
}

function bindSettingDropdowns() {
  for (const d of SETTING_DROPDOWNS) {
    el(d.id).onclick = () => toggleSettingDropdown(d);
  }
  refreshSettingDropdowns();
}

function openSettings() {
  el('set-mark-read').checked = state.settings.mark_read_on_navigate;
  refreshSettingDropdowns();
  // 字体：滑块位置/标签对齐已保存值（启动时已应用过，弹层里再对一次不会闪）
  paintFontControls(state.settings);
  prefetchFontFamilies();
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
}

async function boot() {
  // 点击右键菜单以外的区域时关闭菜单（菜单内部点击不受影响）。
  // 下拉触发按钮同样不算「外面」：否则开菜单的那一次点击冒泡到这里就把它关掉了
  // （菜单在同一事件里被创建又被删除，表现成「下拉点了没反应」——语言/主题/刷新
  // 间隔/字体四个自绘下拉全中招）；「再点同一个下拉 = 关闭」由 toggleSettingDropdown 管。
  document.addEventListener('click', (e) => {
    if (el('ctx-menu') && !e.target.closest('#ctx-menu') && !e.target.closest('.setting-dropdown')) {
      closeContextMenu();
    }
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

  el('settings-close').onclick = () => el('settings-overlay').classList.add('hidden');

  // 左栏分类切换（事件委派：以后加分类不用改这里）
  el('settings-nav-list').addEventListener('click', (e) => {
    const li = e.target.closest('li[data-pane]');
    if (!li) return;
    currentPane = li.dataset.pane;
    showPane(currentPane);
    log(`settings pane=${currentPane}`);
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
  el('set-notify-new-articles').addEventListener('change', async (e) => {
    try {
      state.settings = await invoke('set_notify_new_articles', { enabled: e.target.checked });
      e.target.checked = state.settings.notify_new_articles;
      log(`notifyNewArticles=${state.settings.notify_new_articles}`);
    } catch (err) {
      setStatus(t('status.settingFailed', { error: err.message }), true);
      log(`set_notify_new_articles failed: ${err.message}`);
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
    // 编辑弹窗同理：背后的列表/阅读区不该被快捷键推动（Esc 在上面已处理）
    if (!el('feed-edit-overlay').classList.contains('hidden')) return;

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
