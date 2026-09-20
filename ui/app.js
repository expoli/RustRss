// RustRss M0 探针前端：只采集数据，不做业务。
// 采集口径与外部测量（/proc RSS、启动耗时）保持一致，便于横向对比。

const $ = (id) => document.getElementById(id);

/** 通过 Tauri IPC 把一行诊断打到 stdout；IPC 不可用时返回 false，页面上仍可见 */
async function toStdout(tag, payload) {
  const line = `${tag} ${JSON.stringify(payload)}`;
  try {
    if (window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke) {
      await window.__TAURI__.core.invoke('probe_log', { line });
      return true;
    }
  } catch (err) {
    console.warn('probe_log failed:', err);
  }
  return false;
}

/** 绘制相关的环境信息：dpr 与 matchMedia 分辨率探针用来判断分数缩放是否真的生效 */
function collectEnv() {
  let glRenderer = 'no-webgl';
  try {
    const gl = document.createElement('canvas').getContext('webgl');
    if (gl) {
      const ext = gl.getExtension('WEBGL_debug_renderer_info');
      glRenderer = ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
    }
  } catch (err) {
    glRenderer = 'webgl-error: ' + err;
  }

  const dprProbes = [1, 1.25, 1.5, 1.75, 2].filter(
    (d) => window.matchMedia(`(resolution: ${d}dppx)`).matches
  );

  const nav = performance.getEntriesByType('navigation')[0];
  const paints = performance.getEntriesByType('paint');
  const fcp = paints.find((p) => p.name === 'first-contentful-paint');

  return {
    dpr: window.devicePixelRatio,
    dprProbes,
    inner: [window.innerWidth, window.innerHeight],
    outer: [window.outerWidth, window.outerHeight],
    screen: [screen.width, screen.height],
    avail: [screen.availWidth, screen.availHeight],
    visualViewportScale: window.visualViewport ? window.visualViewport.scale : null,
    colorScheme: window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light',
    glRenderer,
    // 应用内可见的首帧时间（毫秒，相对导航开始）；WebKit 未实现 paint timing 时为 null
    domContentLoadedMs: nav ? Math.round(nav.domContentLoadedEventEnd) : null,
    loadEventMs: nav ? Math.round(nav.loadEventEnd) : null,
    firstContentfulPaintMs: fcp ? Math.round(fcp.startTime) : null,
    userAgent: navigator.userAgent,
  };
}

function renderEnv(env) {
  const lines = [
    `devicePixelRatio      ${env.dpr}          （探针命中：${env.dprProbes.join(', ') || '无'} dppx）`,
    `innerWidth x Height   ${env.inner.join(' x ')}`,
    `outerWidth x Height   ${env.outer.join(' x ')}`,
    `screen                ${env.screen.join(' x ')}   avail ${env.avail.join(' x ')}`,
    `visualViewport.scale  ${env.visualViewportScale}`,
    `配色                   ${env.colorScheme}`,
    `GL renderer           ${env.glRenderer}`,
    `首帧（应用内计时）     FCP=${env.firstContentfulPaintMs}ms  DCL=${env.domContentLoadedMs}ms  load=${env.loadEventMs}ms`,
    `UA                    ${env.userAgent}`,
  ];
  $('env').textContent = lines.join('\n');
}

// ---------- 输入法事件 ----------
const imeLog = [];
function pushIme(entry) {
  imeLog.push(entry);
  if (imeLog.length > 80) imeLog.shift();
  $('ime-log').textContent = imeLog
    .map((e) => `${e.t}  ${e.type.padEnd(17)} ${e.detail}`)
    .join('\n');
  $('ime-log').scrollTop = $('ime-log').scrollHeight;

  const sessions = imeLog.filter((e) => e.type === 'compositionend').length;
  const committed = imeLog
    .filter((e) => e.type === 'compositionend')
    .map((e) => e.detail)
    .join('');
  $('ime-summary').textContent = `组合会话 ${sessions} 次 · 上屏「${committed}」`;
}

function attachIme(el, name) {
  const since = performance.now();
  const stamp = () => `+${Math.round(performance.now() - since)}ms`;

  el.addEventListener('compositionstart', (e) =>
    pushIme({ t: stamp(), type: 'compositionstart', detail: `${name} data="${e.data}"` })
  );
  el.addEventListener('compositionupdate', (e) =>
    pushIme({ t: stamp(), type: 'compositionupdate', detail: `${name} data="${e.data}"` })
  );
  el.addEventListener('compositionend', (e) =>
    pushIme({ t: stamp(), type: 'compositionend', detail: `${name} data="${e.data}" → 值="${el.value}"` })
  );
  el.addEventListener('input', (e) =>
    pushIme({
      t: stamp(),
      type: 'input',
      detail: `${name} isComposing=${e.isComposing} inputType=${e.inputType} 值="${el.value}"`,
    })
  );
  el.addEventListener('keydown', (e) =>
    pushIme({
      t: stamp(),
      type: 'keydown',
      detail: `${name} key=${e.key} code=${e.code} isComposing=${e.isComposing}`,
    })
  );
}

function imeSnapshot() {
  return {
    sessions: imeLog.filter((e) => e.type === 'compositionend').length,
    committedText: imeLog
      .filter((e) => e.type === 'compositionend')
      .map((e) => e.detail)
      .join(''),
    events: imeLog.slice(-40),
  };
}

// ---------- 键盘事件实时可见（用于区分「收不到键盘」与「输入法不可用」）----------
let kbdCount = 0;
let kbdLastFlush = -1e9; // 故意用极小值：保证页面刚加载时的第一个按键也会被上报（之前用 0 会把首键吞掉）
function attachGlobalKeyboard() {
  window.addEventListener(
    'keydown',
    (e) => {
      kbdCount += 1;
      const badge = $('kbd-badge');
      if (badge) {
        badge.textContent = `已收到 ${kbdCount} 次按键，最后：key=${e.key} code=${e.code} isComposing=${e.isComposing} 目标=${e.target.tagName}`;
      }
      const now = performance.now();
      if (now - kbdLastFlush > 300) {
        kbdLastFlush = now;
        toStdout('KEY', { count: kbdCount, key: e.key, code: e.code, composing: e.isComposing, target: e.target.tagName });
      }
    },
    true // 捕获阶段：即使事件最终没落在输入框上也能看到
  );
}

/**
 * 自检：主动派发一个合成 keydown，验证「监听器 → 防抖 → IPC → stdout」这条链路通不通。
 * 它不能证明真键盘事件能到达（那是操作系统/合成器的事），但能排除「探针本身是坏的」这一情形：
 * 只要日志里有 key=SelfTest 的那行，后续「按了键但计数不动」就可以归因到输入通道，而不是探针。
 */
function keyboardSelfTest() {
  const before = kbdCount;
  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'SelfTest', code: 'SelfTest', bubbles: true }));
  const ok = kbdCount > before;
  $('env').textContent += `\n键盘采集链路自检      ${ok ? '通过（keydown 监听与上报链路可用）' : '失败（监听器没被触发，探针本身有问题）'}`;
  // 显式上报自检结果：不依赖防抖逻辑，确保外部能判定「探针可用」
  toStdout('KBD-SELFTEST', { ok, count: kbdCount });
  return ok;
}

// ---------- 启动 ----------
window.addEventListener('DOMContentLoaded', () => {
  const env = collectEnv();
  renderEnv(env);

  attachIme($('ime-input'), 'input');
  attachIme($('ime-area'), 'textarea');
  attachGlobalKeyboard();
  keyboardSelfTest();

  // 首帧计时可能晚于 DOMContentLoaded 才可用，稍后重采一次
  setTimeout(() => {
    const later = collectEnv();
    renderEnv(later);
    toStdout('ENV', later);
  }, 1500);

  // 改 KDE 缩放会触发 resize：这时重采一次，正好拿到缩放变化后的 dpr
  let resizeTimer = null;
  window.addEventListener('resize', () => {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(() => {
      const after = collectEnv();
      renderEnv(after);
      toStdout('RESIZE', after);
    }, 400);
  });

  // 窗口 / 显示器状态：多屏、缩放场景下定位窗口到底落在哪里
  // （多屏时窗口可能开在你没在看的屏上，这一步把事实打出来而不是靠猜）
  (async () => {
    try {
      const info = JSON.parse(await window.__TAURI__.core.invoke('win_info'));
      await toStdout('WIN', info);
      $('env').textContent += `\n窗口(物理像素)        visible=${info.visible} minimized=${info.minimized} pos=${JSON.stringify(info.pos_phys)} outer=${JSON.stringify(info.outer_phys)} scale=${info.scale_factor}`;
      $('env').textContent += `\n当前显示器            ${JSON.stringify(info.monitor)}`;
      $('env').textContent += `\n全部显示器            ${JSON.stringify(info.monitors)}`;
      await window.__TAURI__.core.invoke('win_focus');
      const after = JSON.parse(await window.__TAURI__.core.invoke('win_info'));
      await toStdout('WIN-AFTER-FOCUS', after);
    } catch (err) {
      $('env').textContent += `\n窗口信息              获取失败: ${err}`;
    }
  })();

  // 鼠标通道自检：只靠点击，不依赖键盘；我这边能收到上报即证明鼠标事件到达了应用
  $('mouse-check').addEventListener('click', async () => {
    const ok = await toStdout('MOUSE', { at: Date.now() });
    $('mouse-out').textContent = ok ? '已上报到终端' : 'IPC 不可用';
  });

  $('ime-flush').addEventListener('click', async () => {
    const ok = await toStdout('IME', imeSnapshot());    $('ime-summary').textContent += ok ? ' · 已打到终端' : ' · IPC 不可用（请手工复制）';
  });

  $('ime-clear').addEventListener('click', () => {
    imeLog.length = 0;
    $('ime-log').textContent = '（事件日志）';
    $('ime-summary').textContent = '尚未输入';
  });

  $('measure').addEventListener('click', async () => {
    const m = {
      ...collectEnv(),
      rulerRect: rectOf($('ruler')),
      box100: rectOf(document.querySelector('.blur-test')),
    };
    const ok = await toStdout('SCALE', m);
    $('measure-out').textContent = ok ? '已打到终端' : 'IPC 不可用，请手工复制';
    $('dump-out').value = JSON.stringify(m, null, 2);
  });

  $('dump').addEventListener('click', () => {
    const all = { env: collectEnv(), ime: imeSnapshot(), log: imeLog.length };
    $('dump-out').value = JSON.stringify(all, null, 2);
    $('dump-state').textContent = '已生成';
    toStdout('DUMP', all);
  });

  // ---------- 剪贴板：三条路径独立验证 ----------
  $('clip-web').addEventListener('click', async () => {
    const r = await clipTestWeb();
    clipLog(`① Web API（navigator.clipboard）: ${r.ok ? '写入成功' : '失败 → ' + r.err}`);
    toStdout('CLIP-WEB', r);
  });
  $('clip-exec').addEventListener('click', async () => {
    const r = await clipTestExec();
    clipLog(`② execCommand 兜底: ${r.ok ? '写入成功' : '失败 → ' + r.err}`);
    toStdout('CLIP-EXEC', r);
  });
  $('clip-tauri').addEventListener('click', async () => {
    const r = await clipTestTauri();
    clipLog(`③ Tauri 插件（Rust 侧）: ${r.ok ? '写入成功' : '失败 → ' + r.err}`);
    toStdout('CLIP-TAURI', r);
  });
  $('clip-read').addEventListener('click', async () => {
    const r = await clipReadBack();
    clipLog(`读回：web=${JSON.stringify(r.web)}  tauri=${JSON.stringify(r.tauri)}`);
    toStdout('CLIP-READ', r);
  });

  // 逐条验证：每写一条后立即用插件（独立通道）读回，避免“报成功但没落地”
  $('clip-verify').addEventListener('click', async () => {
    const readBack = async () => {
      try {
        return await window.__TAURI__.core.invoke('clip_read');
      } catch (e) {
        return 'ERR ' + String(e);
      }
    };
    const rows = [];

    const tWeb = CLIP_TEXT('WEB');
    let wWeb;
    try {
      await navigator.clipboard.writeText(tWeb);
      wWeb = 'API 报告成功';
    } catch (e) {
      wWeb = 'API 报错 ' + String(e);
    }
    const rWeb = await readBack();
    rows.push(`① Web API    : ${wWeb}｜剪贴板实际=${JSON.stringify(rWeb).slice(0, 44)}｜落地=${rWeb === tWeb}`);

    const tExec = CLIP_TEXT('EXEC');
    let wExec;
    try {
      const ta = document.createElement('textarea');
      ta.value = tExec;
      ta.style.position = 'fixed';
      ta.style.opacity = '0';
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand('copy');
      document.body.removeChild(ta);
      wExec = ok ? '返回 true' : '返回 false';
    } catch (e) {
      wExec = '抛错 ' + String(e);
    }
    const rExec = await readBack();
    rows.push(`② execCommand: ${wExec}｜剪贴板实际=${JSON.stringify(rExec).slice(0, 44)}｜落地=${rExec === tExec}`);

    const tTauri = CLIP_TEXT('TAURI');
    let wTauri;
    try {
      await window.__TAURI__.core.invoke('clip_write', { text: tTauri });
      wTauri = '调用成功';
    } catch (e) {
      wTauri = '报错 ' + String(e);
    }
    const rTauri = await readBack();
    rows.push(`③ Tauri 插件 : ${wTauri}｜剪贴板实际=${JSON.stringify(rTauri).slice(0, 44)}｜落地=${rTauri === tTauri}`);

    rows.forEach(clipLog);
    toStdout('CLIP-VERIFY', { rows });
  });
});

function rectOf(el) {
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { x: r.x, y: r.y, w: r.width, h: r.height };
}

// ---------- 剪贴板自检：每条路径独立验证，互不掩盖 ----------
const CLIP_TEXT = (tag) => `RUSTSS-CLIP-${tag}-${Date.now()}`;

function clipLog(msg) {
  const el = $('clip-out');
  el.textContent += `\n${msg}`;
  el.scrollTop = el.scrollHeight;
}

/** 路径①：WebKit 的 Web Clipboard API（产品里不应当依赖它） */
async function clipTestWeb() {
  if (!navigator.clipboard || !navigator.clipboard.writeText) {
    return { path: 'web:navigator.clipboard', ok: false, err: 'API 不存在' };
  }
  try {
    await navigator.clipboard.writeText(CLIP_TEXT('WEB'));
    return { path: 'web:navigator.clipboard', ok: true };
  } catch (e) {
    return { path: 'web:navigator.clipboard', ok: false, err: String(e) };
  }
}

/** 路径②：旧的 execCommand('copy') 兜底 */
async function clipTestExec() {
  try {
    const ta = document.createElement('textarea');
    ta.value = CLIP_TEXT('EXEC');
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand('copy');
    document.body.removeChild(ta);
    return { path: 'execCommand', ok, err: ok ? null : 'execCommand 返回 false' };
  } catch (e) {
    return { path: 'execCommand', ok: false, err: String(e) };
  }
}

/** 路径③：Tauri 剪贴板插件（Rust 侧），产品里应当走这条 */
async function clipTestTauri() {
  try {
    if (!window.__TAURI__ || !window.__TAURI__.core) {
      return { path: 'tauri:clipboard-manager', ok: false, err: 'IPC 不可用' };
    }
    await window.__TAURI__.core.invoke('clip_write', { text: CLIP_TEXT('TAURI') });
    return { path: 'tauri:clipboard-manager', ok: true };
  } catch (e) {
    return { path: 'tauri:clipboard-manager', ok: false, err: String(e) };
  }
}

/** 读回系统剪贴板：用来确认“写入成功”是否真的落到了系统剪贴板 */
async function clipReadBack() {
  const out = { web: null, tauri: null };
  try {
    out.web = navigator.clipboard && navigator.clipboard.readText
      ? await navigator.clipboard.readText()
      : 'API 不存在';
  } catch (e) {
    out.web = 'ERR ' + String(e);
  }
  try {
    out.tauri = await window.__TAURI__.core.invoke('clip_read');
  } catch (e) {
    out.tauri = 'ERR ' + String(e);
  }
  return out;
}
