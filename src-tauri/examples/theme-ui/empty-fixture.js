// Test-only driver: the three fixed scenes rendered with EMPTY data, using the
// same production components and CSS as fixture.js.
//
// Why it exists: PRD line 47 requires the three fixed scenes (overview / article /
// settings) to include empty and error states, and the main matrix rendered 30
// entries plus two feeds, so the empty half of that requirement was never
// captured. This fixture renders the empties with production markup copied from
// ui/app.js (renderList's empty branch, renderReaderEmpty, and the failed-feed
// dot/tooltip), then captures the same 3 scenes x 3 presets x 2 modes x 2 locales
// through the native WebView snapshot path.
(async () => {
  const invoke = window.__TAURI__.core.invoke;
  const report = {
    fixture_version: 1,
    fixture: 'empty',
    captures: [],
    checks: [],
    empty_copy: {},
    fonts: 'requested/computed stacks only; glyph fallback not inferred',
  };
  const assert = (ok, name) => { if (!ok) throw new Error(name); report.checks.push(name); };
  const el = (id) => document.getElementById(id);
  // The settings surface colour must come from the DOM, not from a token name: later
  // batches changed which token the pane paints, and a stale token made the old
  // coordinate check pass by coincidence.
  function paneSurface() {
    // The opaque surface belongs to the dialog, not to the pane div.
    const surfaceEl = el('pane-appearance').closest('.settings-dialog') || el('pane-appearance');
    const raw = getComputedStyle(surfaceEl).backgroundColor;
    const m = /^rgba?\((\d+), (\d+), (\d+)(?:, ([\d.]+))?\)$/.exec(raw);
    if (!m || (m[4] !== undefined && Number(m[4]) < 0.9)) throw new Error(`settings pane surface is not opaque: ${raw}`);
    const parts = [m[1], m[2], m[3]].map((n) => Number(n).toString(16).padStart(2, '0'));
    const hex = '#' + parts.join('');
    const lum = (0.2126 * Number(m[1]) + 0.7152 * Number(m[2]) + 0.0722 * Number(m[3])) / 255;
    return { hex, lum };
  }
  const settle = async () => {
    await document.fonts.ready;
    await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
  };

  // ui/app.js buildFeedRow: the failed flag drives the dot and the tooltip.
  const FAILED_STATUS = 'http_503';
  function feedRow(failed) {
    const li = document.createElement('li');
    li.className = 'folder-feed';
    li.innerHTML = RustRssComponents.feedContent();
    li.querySelector('.name').textContent = failed ? 'Unavailable feed · 错误状态' : 'Design notes · 设计';
    li.querySelector('.dot').hidden = !failed;
    li.querySelector('.count').textContent = '0';
    li.title = failed
      ? I18N.t('sidebar.feedTooltipFailed', { status: FAILED_STATUS, error: I18N.t('fetchError.http', { status: '503' }) })
      : I18N.t('sidebar.feedTooltipOk', { url: 'https://example.invalid/feed.xml' });
    return li;
  }

  function buildEmptyScene(locale) {
    I18N.setLocale(locale);
    I18N.applyStaticI18n();
    el('views').innerHTML = '';
    // ui/app.js VIEWS: kind, i18n key, icon. Counts are zero because this is the empty matrix.
    for (const [icon, key] of [['●', 'list.unread'], ['★', 'list.starred'], ['⏱', 'list.later'], ['≡', 'list.all']]) {
      const li = document.createElement('li');
      li.innerHTML = RustRssComponents.viewContent(icon);
      li.querySelector('.vlabel').textContent = I18N.t(key);
      li.querySelector('.count').textContent = '0';
      el('views').append(li);
    }
    // The error state lives in the sidebar next to the empties on purpose: the
    // requirement is that both are visible in the fixed scenes.
    el('feeds').innerHTML = '';
    el('feeds').append(feedRow(true));
    el('list-title').textContent = I18N.t('list.unread');
    el('list-count').textContent = '0';
    // ui/app.js renderList: an empty view is one dim row with the empty copy.
    el('entries').innerHTML = '';
    for (const key of ['list.emptySubscriptions', 'list.empty']) {
      const li = document.createElement('li');
      li.className = 'dim';
      li.style.cursor = 'default';
      li.textContent = I18N.t(key);
      el('entries').append(li);
    }
    // ui/app.js renderReaderEmpty: three paragraphs, not one.
    el('reader').innerHTML = `<div class="reader-empty">
      <p>${I18N.t('reader.empty')}</p>
      <p class="dim">${I18N.t('reader.shortcuts')}</p>
      <p class="dim">${I18N.t('tags.semantics')}</p>
    </div>`;
  }

  try {
    let editor;
    el('pane-appearance').classList.remove('hidden');
    const snapshots = await invoke('snapshots');
    const renderer = RustRssTheme.createRenderer(document.documentElement, { reader: el('reader'), list: el('entries') });
    assert(I18N.selfTest().ok, 'i18n key sets and markup');

    for (const locale of ['zh-CN', 'en']) {
      buildEmptyScene(locale);
      const empties = [...el('entries').querySelectorAll('li')].map((li) => li.textContent);
      assert(empties[0] === I18N.t('list.emptySubscriptions') && empties[1] === I18N.t('list.empty'),
        `list empty copy ${locale}`);
      assert(el('reader').querySelector('.reader-empty p').textContent === I18N.t('reader.empty'),
        `reader empty copy ${locale}`);
      assert(el('reader').querySelectorAll('.reader-empty p').length === 3, `reader empty paragraphs ${locale}`);
      const failedRow = el('feeds').firstElementChild;
      assert(failedRow.querySelector('.dot').hidden === false && failedRow.title.length > 0, `failed feed state ${locale}`);
      assert(el('entries').children.length === 2 && el('entries').querySelector('.entry') === null, `no article rows ${locale}`);
      report.empty_copy[locale] = { list: empties[0], reader: el('reader').querySelector('.reader-empty p').textContent,
        tooltip: failedRow.title };
      for (const [i, snapshot] of snapshots.entries()) {
        for (const mode of ['light', 'dark']) {
          snapshot.config.mode = mode;
          renderer.apply(snapshot);
          const v = snapshot[mode];
          editor?.dispose();
          editor = RustRssThemeSettings.createEditor(el('appearance-editor'), { kind: 'appearance', getSnapshot: () => snapshot, invoke, apply: () => {}, t: I18N.t });
          for (const scene of ['overview', 'article', 'settings']) {
            el('reader').style.visibility = scene === 'overview' ? 'hidden' : '';
            el('settings-overlay').classList.toggle('hidden', scene !== 'settings');
            await settle();
            // The empty copy must still be the rendered text in every cell of the
            // matrix, not just in the first one.
            assert(el('reader').querySelector('.reader-empty p').textContent === I18N.t('reader.empty'),
              `reader empty copy kept ${locale}/${mode}/${scene}`);
            const name = `empty-${['clear', 'paper', 'slate'][i]}-${mode}-${locale}-${scene}`;
            const shot = { ...await invoke('capture_scene', { name }), name, background: v.colors.background };
            if (scene === 'settings') {
              const probeX = Math.round(el('entries').getBoundingClientRect().right - 5);
              const top = document.elementFromPoint(probeX, 150);
              assert(!el('entries').contains(top) && !document.querySelector('.sidebar').contains(top),
                `settings overlay covers list region ${locale}/${mode}/${scene}`);
              const surface = paneSurface();
              assert(mode === 'light' ? surface.lum > 0.5 : surface.lum < 0.5, `settings surface follows ${mode} ${locale}`);
              Object.assign(shot, { settings_background: surface.hex, occlusion_point: [probeX, 150] });
            }
            report.captures.push(shot);
          }
        }
      }
    }
    const [zh, en] = [report.empty_copy['zh-CN'], report.empty_copy.en];
    assert(zh.list !== en.list && zh.reader !== en.reader && zh.tooltip !== en.tooltip, 'empty copy differs between locales');
    el('settings-overlay').classList.add('hidden');
    el('reader').style.visibility = '';
    report.viewport = [innerWidth, innerHeight];
    report.dpr = devicePixelRatio;
  } catch (error) {
    report.error = String(error.stack || error);
  }
  await invoke('finish', { report });
})();
