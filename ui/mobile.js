// 窄屏（手机/平板）页面式导航适配层：Articles / Subscriptions / Saved / Settings
// 四个一级目的地 + 全屏阅读器。宽触屏（>960px）下本模块整体休眠，DOM 与
// 行为完全不变；所有表现差异都由 body[data-mpage] + CSS 媒体查询驱动。
//
// 设计约束：app.js 是 IIFE，内部状态（state/setView）不可从外部触及——本模块因此
// 做成**纯 DOM 适配层**：目的地/分段切换通过程序化点击侧栏既有行（同一套 setView），
// 阅读器与弹层的状态用 MutationObserver 观察，Android 系统返回键走 History API。
// Android WebView 的 canGoBack() 有时不计入 pushState 条目，因此通过 Tauri 的
// back-button 事件显式调用 history.back()。列表页在阅读页打开期间保持渲染（页面用覆盖层
// 盖住而非 display:none），因此过滤器、选中项与滚动位置天然原样保留。
(function () {
  'use strict';

  var MQ = window.matchMedia('(max-width: 960px) and (pointer: coarse)');
  var active = function () { return MQ.matches; };

  // 阅读器打开前的目的地：popstate 恢复时回到哪里。
  var returnPage = 'articles';
  var settingsReturnPage = 'articles';
  // History 栈的镜像：[{ t: 'reader', returnPage } | { t: 'ovl', id, closeSel }]。
  // 不依赖 event.state（各实现交付时机不一致），自己记 LIFO。
  var historyStack = [];
  // 我们自己调用 history.back() 消费已关闭弹层的条目时置位，避免 popstate 把它
  // 当成一次用户返回再处理。
  var consuming = false;
  var menuBackFromHistory = false;
  var pendingMenuAction = null;
  // 长按已触发时抑制随后的 click（长按菜单打开后松手不应再走到导航/打开文章）。
  var longPressFired = false;

  function el(id) { return document.getElementById(id); }
  function bodyPage() { return document.body.dataset.mpage || null; }

  function setPage(name) {
    if (name !== bodyPage()) window.RustRssMenu?.close();
    document.body.dataset.mpage = name;
    // The reader visually covers the list, so remove that background page from
    // TalkBack and keyboard traversal while keeping its DOM/scroll anchor live.
    document.querySelector('main > .list').inert = name === 'reader';
    syncNav();
    syncSegments();
    window.RustRssLocation?.page(name);
  }

  function syncNav() {
    var page = bodyPage();
    document.querySelectorAll('#m-nav .m-nav-btn').forEach(function (btn) {
      var name = btn.dataset.mpageBtn;
      var on = name === page;
      btn.classList.toggle('active', on);
      btn.setAttribute('aria-current', on ? 'page' : 'false');
    });
  }

  // 分段切换的高亮跟 #views 行的 active 同步（那一行是权威：setView 由它触发）。
  function syncSegments() {
    var current = (document.querySelector('#views li.active') || {}).dataset || {};
    document.querySelectorAll('.m-seg [data-mview]').forEach(function (btn) {
      btn.classList.toggle('active', btn.dataset.mview === current.kind);
      btn.setAttribute('aria-pressed', String(btn.dataset.mview === current.kind));
    });
  }

  // 分段切换 = 点击侧栏视图行（app.js 的 setView 全走那一条路，别处不再有第二份过滤逻辑）。
  function pickView(kind) {
    var row = document.querySelector('#views li[data-kind="' + kind + '"]');
    if (row) row.click();
  }

  // ---- 一级目的地 -------------------------------------------------------

  function pickDestination(name) {
    if (!active()) return;
    if (name === 'settings') {
      // 复用设置初始化/草稿生命周期，页面和返回栈由观察器同步。
      el('btn-settings').click();
      return;
    }
    if (!el('settings-overlay').classList.contains('hidden')) el('settings-close').click();
    var kind = (document.querySelector('#views li.active') || {}).dataset?.kind;
    if (name === 'articles' && (kind === 'starred' || kind === 'later')) {
      pickView('unread'); // 收藏视图不属于文章目的地：切回默认未读
    } else if (name === 'saved' && kind !== 'starred' && kind !== 'later') {
      pickView(savedMode);
    }
    if (name === 'digest') {
      // 日报页（设计 2026-10-04 #4）：今日/昨日/近期历史的日期列表。
      // 渲染进 .right-col（与 reader 覆盖层同容器）；先退阅读层再进。
      setPage('digest');
      if (digestMode === 'chat') window.RustRssChatBridge?.resume();
      else renderDigestHome();
      return;
    }
    setPage(name);
  }

  var digestMode = 'digest';

  function syncDigestTabs() {
    document.querySelectorAll('[data-digest-tab]').forEach(function (button) {
      var selected = button.dataset.digestTab === digestMode;
      button.classList.toggle('active', selected);
      button.setAttribute('aria-pressed', String(selected));
    });
  }

  var savedMode = 'starred'; // 收藏页内部的 星标/稍后读 记忆（会话内）

  // ---- History / Android 返回键 ----------------------------------------

  function pushEntry(entry) {
    historyStack.push(entry);
    history.pushState({ rr: historyStack.length }, '');
  }

  function watchAndroidBack() {
    if (document.body.dataset.android !== '1') return;
    var app = window.__TAURI__?.app;
    if (!app?.onBackButtonPress) return;
    app.onBackButtonPress(function () {
      // Keep chat drafts/tasks alive: close IME first, then return to digests.
      if (active() && document.activeElement?.id === 'chat-input') {
        document.activeElement.blur();
        return;
      }
      if (active() && historyStack.length) history.back();
      else if (active() && bodyPage() === 'digest' && digestMode === 'chat') renderDigestHome();
      else app.exit();
    });
  }

  window.addEventListener('popstate', function () {
    if (consuming) {
      consuming = false;
      if (pendingMenuAction) {
        const action = pendingMenuAction;
        pendingMenuAction = null;
        window.RustRssMenu?.runAction(action);
      }
      return;
    }
    if (!active()) { historyStack = []; return; }
    var entry = historyStack.pop();
    if (!entry) return; // 外来导航（理论不会有）：忽略
    if (entry.t === 'reader') {
      setPage(entry.returnPage);
      // 日报详情占用了 right-col：返回日报页时重渲染日期列表
      if (entry.returnPage === 'digest') renderDigestHome();
    } else if (entry.t === 'menu-page') {
      menuBackFromHistory = true;
      window.RustRssMenu?.back();
      menuBackFromHistory = false;
    } else if (entry.t === 'menu') {
      window.RustRssMenu?.close();
    } else if (entry.t === 'ovl') {
      var closer = el(entry.id) && el(entry.id).querySelector(entry.closeSel);
      if (closer) closer.click();
    } else if (entry.t === 'settings-pane') {
      el('m-settings-back').click();
    }
  });

  window.addEventListener('rustrss-menu-open', function () {
    if (active()) pushEntry({ t: 'menu' });
  });
  window.addEventListener('rustrss-menu-page', function () {
    if (active()) pushEntry({ t: 'menu-page' });
  });
  window.addEventListener('rustrss-menu-back', function () {
    if (!active() || menuBackFromHistory) return;
    if (historyStack.at(-1)?.t === 'menu-page') {
      historyStack.pop(); consuming = true; history.back();
    }
  });
  window.addEventListener('rustrss-menu-close', function () {
    longPressFired = false;
    if (!active()) return;
    const index = historyStack.findLastIndex(function (entry) { return entry.t === 'menu'; });
    if (index < 0) return;
    const count = historyStack.length - index;
    historyStack.splice(index);
    consuming = true; history.go(-count);
  });
  window.addEventListener('rustrss-menu-action', function (event) {
    pendingMenuAction = event.detail;
  });

  // 阅读器：renderReader 会整体替换 #reader 内容，观察「正文头部出现」这一刻。
  // 已在阅读页时的重渲染（抓全文等）不重复压栈。
  function watchReader() {
    var reader = el('reader');
    new MutationObserver(function () {
      if (!active() || !reader.querySelector('.reader-head')) return;
      if (bodyPage() === 'reader') return;
      returnPage = bodyPage() || 'articles';
      pushEntry({ t: 'reader', returnPage: returnPage });
      setPage('reader');
    }).observe(reader, { childList: true });
  }

  // 弹层/对话框：出现时压一条历史（系统返回 = 关闭它而不是退出应用），
  // 通过点击各自的关闭按钮关闭（复用应用自己的收尾逻辑，不另写第二份）。
  var OBSERVED_OVERLAYS = [
    { id: 'settings-overlay', closeSel: '#settings-close', attr: 'class', shown: function (n) { return !n.classList.contains('hidden'); }, onToggle: syncNav },
    { id: 'ai-confirm-overlay', closeSel: '#ai-confirm-cancel', attr: 'class', shown: function (n) { return !n.classList.contains('hidden'); } },
    { id: 'generic-confirm-overlay', closeSel: '#generic-confirm-cancel', attr: 'class', shown: function (n) { return !n.classList.contains('hidden'); } },
    { id: 'feed-edit-overlay', closeSel: '#feed-edit-cancel', attr: 'class', shown: function (n) { return !n.classList.contains('hidden'); } },
    { id: 'tag-picker-overlay', closeSel: '#tag-picker-close', attr: 'class', shown: function (n) { return !n.classList.contains('hidden'); } },
    { id: 'aa-dialog', closeSel: '#aa-close', attr: 'open', shown: function (n) { return n.hasAttribute('open'); } },
    { id: 'keyboard-help', closeSel: '#keyboard-help-close', attr: 'open', shown: function (n) { return n.hasAttribute('open'); } },
  ];

  function watchOverlays() {
    OBSERVED_OVERLAYS.forEach(function (ov) {
      var node = el(ov.id);
      if (!node) return;
      new MutationObserver(function () {
        var shown = ov.shown(node);
        if (!active()) return;
        var top = historyStack[historyStack.length - 1];
        var mine = function (e) { return e.t === 'ovl' && e.id === ov.id; };
        if (shown) {
          if (ov.id === 'settings-overlay' && bodyPage() !== 'settings') {
            settingsReturnPage = bodyPage() || 'articles';
            setPage('settings');
          }
          syncNav();
          if (!top || !mine(top)) pushEntry({ t: 'ovl', id: ov.id, closeSel: ov.closeSel });
        } else {
          if (ov.id === 'settings-overlay' && bodyPage() === 'settings') setPage(settingsReturnPage);
          syncNav();
          var index = historyStack.findLastIndex(mine);
          if (index >= 0) {
            // 弹层经自己的按钮关闭：消费掉那条历史，让下一次返回不被它占住。
            // 设置还可能有一条详情历史，关闭时一起消费。
            var count = historyStack.length - index;
            historyStack.splice(index);
            consuming = true;
            history.go(-count);
          }
        }
        if (ov.onToggle) ov.onToggle();
      }).observe(node, { attributes: true, attributeFilter: [ov.attr] });
    });
  }

  function watchSettingsNavigation() {
    var dialog = document.querySelector('.settings-dialog');
    new MutationObserver(function () {
      if (!active() || el('settings-overlay').classList.contains('hidden')) return;
      var top = historyStack[historyStack.length - 1];
      if (dialog.dataset.screen === 'detail') {
        if (!top || top.t !== 'settings-pane') pushEntry({ t: 'settings-pane' });
      } else if (top && top.t === 'settings-pane') {
        historyStack.pop(); consuming = true; history.back();
      }
    }).observe(dialog, { attributes: true, attributeFilter: ['data-screen'] });
  }

  // 原生层负责 IME 避让；WebView 缩小后让当前输入滚进可用区域。
  // 只滚动输入所在容器，不重建列表，也不改变阅读状态。
  function keepInputVisible() {
    var pending;
    var reveal = function () {
      clearTimeout(pending);
      pending = setTimeout(function () {
        var input = document.activeElement;
        if (active() && input?.matches('input, textarea, select')) {
          input.scrollIntoView({ block: 'nearest', inline: 'nearest' });
        }
      }, 120);
    };
    window.addEventListener('resize', reveal);
    window.visualViewport?.addEventListener('resize', reveal);
    document.addEventListener('focusin', reveal);
  }

  // ---- 状态镜像 ---------------------------------------------------------

  // 桌面状态栏在右栏底部，窄屏下列表/订阅页看不到它。把 #status 的文本与错误态
  // 同步到一条固定镜像（只搬文本，不复制行为）。
  function watchStatus() {
    var status = el('status');
    var mirror = el('m-status-text');
    var sync = function () {
      mirror.textContent = status.textContent;
      mirror.classList.toggle('error', status.classList.contains('error'));
    };
    new MutationObserver(sync).observe(status, { childList: true, characterData: true, subtree: true });
    new MutationObserver(sync).observe(status, { attributes: true, attributeFilter: ['class'] });
    sync();
  }

  // ---- 分段切换与侧栏联动 ----------------------------------------------

  function watchViewsActive() {
    new MutationObserver(syncSegments).observe(el('views'), { subtree: true, attributes: true, attributeFilter: ['class'] });
  }

  // 订阅页里点 订阅源/文件夹/标签 行 → 切到文章页显示过滤后的列表。
  // app.js 的容器级 click 委托负责真正的 setView，这里只搬页面（顺序无关）。
  function watchSidebarSelection() {
    document.addEventListener('click', function (ev) {
      if (!active() || bodyPage() !== 'subscriptions') return;
      var feed = ev.target.closest('#feeds li[data-feed-id]');
      var head = ev.target.closest('#feeds li.folder-head');
      var tag = ev.target.closest('#tags li[data-tag-id]');
      var view = ev.target.closest('#views li[data-kind]');
      var arrow = ev.target.closest('.folder-arrow');
      if (ev.target.closest('.row-more')) return;
      if ((feed || tag || view) && !arrow) setPage('articles');
      else if (head && !arrow) setPage('articles'); // 文件夹头（非折叠箭头）= 过滤到该文件夹
    }, true);
  }

  // ---- 长按 → 右键菜单（触屏上的管理动作入口） -------------------------

  function initLongPress() {
    var timer = null;
    var start = null;
    var target = null;
    var MENU_HOLD_MS = 550;
    var MOVE_TOLERANCE = 12;

    function cancel() {
      if (timer) { clearTimeout(timer); timer = null; }
      start = null; target = null;
    }

    document.addEventListener('pointerdown', function (ev) {
      if (!active() || ev.pointerType !== 'touch') return;
      var row = ev.target.closest('#feeds li, #tags li');
      if (ev.target.closest('.row-more')) return;
      if (!row) return;
      target = row;
      start = { x: ev.clientX, y: ev.clientY };
      timer = setTimeout(function () {
        timer = null;
        if (!start || !target) return;
        longPressFired = true;
        target.dispatchEvent(new MouseEvent('contextmenu', {
          bubbles: true, cancelable: true,
          clientX: start.x, clientY: start.y,
        }));
      }, MENU_HOLD_MS);
    }, { passive: true });

    document.addEventListener('pointermove', function (ev) {
      if (!start) return;
      if (Math.abs(ev.clientX - start.x) > MOVE_TOLERANCE || Math.abs(ev.clientY - start.y) > MOVE_TOLERANCE) cancel();
    }, { passive: true });

    document.addEventListener('pointerup', cancel, { passive: true });
    document.addEventListener('pointercancel', cancel, { passive: true });
    document.addEventListener('scroll', cancel, { passive: true, capture: true });

    // 长按后的那次 click 不再派发（否则菜单开了又被行的默认动作打断）。
    document.addEventListener('click', function (ev) {
      if (!longPressFired) return;
      longPressFired = false;
      // A WebView may omit the release click after a long press. In that case
      // the next click can be an intentional action inside the new sheet.
      if (ev.target.closest('#ctx-menu')) return;
      ev.stopPropagation();
      ev.preventDefault();
    }, true);
  }

  // ---- 宽度档位切换 -----------------------------------------------------

  function applyMode() {
    el('settings-overlay').setAttribute('role', active() ? 'region' : 'dialog');
    el('settings-overlay').setAttribute('aria-modal', String(!active()));
    document.querySelectorAll('.m-setting-advanced').forEach(function (details) { details.open = !active(); });
    if (active()) {
      // 同一设置 DOM 放入主内容区，底栏是它的兄弟；桌面仍使用原模态窗口。
      document.querySelector('main').append(el('settings-overlay'));
      if (!bodyPage()) setPage('articles');
      // 保留同一套订阅行为，只搬动表单；手机入口常驻上方。
      el('views').after(el('add-row'));
      el('add-row').classList.remove('hidden');
    } else {
      document.body.append(el('settings-overlay'));
      historyStack = [];
      delete document.body.dataset.mpage;
      document.querySelector('main > .list').inert = false;
      document.querySelector('.sidebar-foot').before(el('add-row'));
      el('add-row').classList.add('hidden');
    }
  }

  // ---- 日报页（设计 2026-10-04 #4） ------------------------------------
  /// 日报首页：今日/昨日/近期历史（digest_list 有界元数据），点击进阅读层。
  function renderDigestHome() {
    digestMode = 'digest';
    syncDigestTabs();
    window.RustRssChatBridge?.leave();
    window.RustRssChatBridge?.invalidateReader();
    window.RustRssLocation?.digestHome();
    var t = window.I18N ? window.I18N.t : function (k) { return k; };
    try {
    var bridge = window.RustRssDigestBridge || {};
    var days = bridge.days ? bridge.days() : [];
    var list = [
      { label: t('sidebar.digestToday'), kind: 'today' },
      { label: t('sidebar.digestYesterday'), kind: 'yesterday' },
    ];
    var today = bridge.date ? bridge.date('today') : '';
    var yesterday = bridge.date ? bridge.date('yesterday') : '';
    var history = days.filter(function (d) {
      return d.date !== today && d.date !== yesterday;
    });
    var html = '<div class="m-digest-home">';
    html += '<h2 class="m-digest-title">' + t('m.nav.digest') + '</h2>';
    list.forEach(function (d) {
      var has = days.some(function (x) { return x.date === (bridge.date ? bridge.date(d.kind) : ''); });
      html += '<button type="button" class="m-digest-row" data-digest-kind="' + d.kind + '">' +
        d.label + (has ? ' <span class="dot">●</span>' : '') + '</button>';
    });
    if (history.length) {
      html += '<h3 class="m-digest-sub">' + t('digest.history') + '</h3>';
      history.forEach(function (d) {
        html += '<button type="button" class="m-digest-row" data-digest-date="' + d.date + '" ' +
          'data-digest-scope="' + (d.scope_key || 'all') + '">' + d.date +
          ' · ' + (d.article_count || 0) + '</button>';
      });
    }
    html += '</div>';
    var host = document.querySelector('.right-col #reader');
    if (!host) return;
    host.classList.add('digest-home');
    host.innerHTML = html;
    host.querySelectorAll('[data-digest-kind]').forEach(function (btn) {
      btn.addEventListener('click', function () { bridge.open(btn.dataset.digestKind); });
    });
    host.querySelectorAll('[data-digest-date]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        bridge.openDate(btn.dataset.digestDate, btn.dataset.digestScope);
      });
    });
    } catch (err) {
      var host2 = document.querySelector('.right-col #reader');
      if (host2) host2.innerHTML = '<div class="reader-empty">digest home error: ' + (err && err.message) + '</div>';
    }
    // 打开日报（openDigest* 渲染进 #reader 并设 reader 覆盖层），返回到日报首页
    // 需要记位：返回按钮逻辑复用 data-mpage="digest" 的呈现。
  }

  // digest 首页渲染入口暴露给 app.js（digest_list 刷新时同步）；
  // onDigestView：日报详情进共享阅读层，返回（#m-reader-back / 系统返回）回日报首页
  window.RustRssMobileDigest = {
    restoreDefault: function () { if (active()) setPage('articles'); },
    restoreHome: function () { if (active()) { digestMode = 'digest'; setPage('digest'); renderDigestHome(); } },
    renderDigestHome: function () { if (active() && bodyPage() === 'digest' && digestMode === 'digest') renderDigestHome(); },
    // 日报详情进共享阅读层：显式压返回栈（returnPage=digest）。watchReader 的
    // 观察器在 bodyPage 已变 reader 后会跳过压栈——这里不压，返回就落 articles。
    onDigestView: function () {
      if (!active()) return;
      returnPage = 'digest';
      if (bodyPage() !== 'reader') pushEntry({ t: 'reader', returnPage: 'digest' });
      setPage('reader');
    },
  };

  window.RustRssMobileChat = {
    onChatView: function () {
      if (!active()) return;
      digestMode = 'chat';
      setPage('digest');
      syncDigestTabs();
      // Discussing a digest leaves its explicit reader return entry in place.
      // Back still returns to the digest list; completion does not navigate.
    },
  };

  // ---- 初始化 -----------------------------------------------------------

  function init() {
    // Android 运行环境标记：桌面/移动共享的界面元素据此显示平台专属提示
    //（如 AI 端点的设备可达指引）。
    if (/Android/i.test(navigator.userAgent)) {
      document.body.dataset.android = '1';
      const hint = document.getElementById('ai-endpoint-android-hint');
      if (hint) hint.hidden = false;
    }
    if (el('m-nav')) {
      document.querySelectorAll('#m-nav .m-nav-btn').forEach(function (btn) {
        btn.addEventListener('click', function () { pickDestination(btn.dataset.mpageBtn); });
      });
    }
    document.querySelectorAll('.m-seg [data-mview]').forEach(function (btn) {
      btn.addEventListener('click', function () {
        if (btn.closest('#m-seg-saved')) savedMode = btn.dataset.mview;
        pickView(btn.dataset.mview);
      });
    });
    document.querySelectorAll('[data-digest-tab]').forEach(function (button) {
      button.addEventListener('click', function () {
        digestMode = button.dataset.digestTab;
        if (digestMode === 'chat') window.RustRssChatBridge?.resume();
        else renderDigestHome();
        syncDigestTabs();
      });
    });
    var readerBack = el('m-reader-back');
    if (readerBack) {
      readerBack.addEventListener('click', function () {
        var top = historyStack[historyStack.length - 1];
        if (top && top.t === 'reader') history.back();
        else setPage(returnPage); // 没有栈底条目时的兜底（理论不达）
      });
    }
    if (el('status')) watchStatus();
    if (el('views')) watchViewsActive();
    watchReader();
    watchOverlays();
    watchSettingsNavigation();
    watchAndroidBack();
    keepInputVisible();
    watchSidebarSelection();
    initLongPress();
    if (MQ.addEventListener) MQ.addEventListener('change', applyMode);
    else MQ.addListener(applyMode);
    applyMode();
    syncSegments();
  }

  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', init);
  else init();
})();
