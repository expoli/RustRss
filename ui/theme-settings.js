// Shared appearance / reading / Aa editor. Core owns validation and persistence.
(function (global) {
  'use strict';
  const fields = [
    ['typography.ui_family', 'appearance', 'font'], ['typography.ui_size', 'appearance', 12, 20, 1],
    ['list.density', 'appearance', ['comfortable', 'compact']], ['list.summary_lines', 'appearance', 0, 3, 1],
    ['list.thumbnail', 'appearance', 'boolean'], ['chrome.radius', 'appearance', 0, 16, 1],
    ['chrome.sidebar_width', 'appearance', 180, 300, 10], ['chrome.list_width', 'appearance', 260, 460, 10],
    ['typography.read_family', 'reading', 'font'], ['typography.read_size', 'reading', 13, 28, 1],
    ['typography.line_height', 'reading', 1.3, 2.2, 0.05], ['typography.mono_family', 'reading', 'font'],
    ['typography.mono_size', 'reading', 12, 24, 1], ['reader.width', 'reading', 480, 960, 20],
    ['reader.paragraph_gap', 'reading', 0.5, 2, 0.1], ['reader.layout', 'reading', ['three_column', 'focus']],
  ];
  function put(patch, path, value) {
    const keys = path.split('.'); let node = patch;
    for (const key of keys.slice(0, -1)) { if (!node[key] || typeof node[key] !== 'object') node[key] = {}; node = node[key]; }
    node[keys.at(-1)] = value;
    return patch;
  }
  function valueAt(value, path) { return path.split('.').reduce((v, key) => v?.[key], value); }
  function parseField(field, value) {
    if (field[2] === 'font') return value.split(',').map(v => v.trim()).filter(Boolean);
    if (field[2] === 'boolean') return !!value;
    if (typeof field[2] === 'number') return Number(value);
    return value;
  }
  // Token invalidates late validation replies after another edit, cancel, or reload.
  function draftSession(snapshot, validate) {
    let base = snapshot, patch = {}, generation = 0;
    return {
      get base() { return base; }, get patch() { return structuredClone(patch); },
      get dirty() { return Object.keys(patch).length > 0; },
      set(path, value) {
        // Clearing all overrides followed by one edit must still clear the other
        // old leaves, rather than silently turn the reset back into a sparse edit.
        if (path.startsWith('overrides.') && patch.overrides === null) {
          const cleared = value => value && !Array.isArray(value) && typeof value === 'object'
            ? Object.fromEntries(Object.entries(value).map(([k, v]) => [k, cleared(v)])) : null;
          patch.overrides = cleared(base.config.overrides || {});
        }
        put(patch, path, value); generation++;
      },
      reset(next) { base = next; patch = {}; generation++; },
      async preview() {
        const token = ++generation;
        try {
          const result = await validate(base.config.revision, structuredClone(patch));
          return token === generation ? result : null;
        } catch (err) { if (token === generation) throw err; return null; }
      },
    };
  }
  function createEditor(host, { kind, getSnapshot, invoke, apply, t, fontNames = () => [] }) {
    const session = draftSession(getSnapshot(), (expectedRevision, patch) => invoke('validate_ui_theme', { expectedRevision, patch }));
    let busy = false, disposed = false, previewMode = 'light', renderer, controls = [];
    const node = (tag, text, cls) => { const e = document.createElement(tag); if (text) e.textContent = text; if (cls) e.className = cls; return e; };
    const button = (text, action) => { const b = node('button', text); b.type = 'button'; b.onclick = action; return b; };
    const status = node('p', '', 'theme-editor-status'); status.setAttribute('role', 'status');
    const form = node('fieldset', '', 'theme-editor-fields');
    const sample = node('section', '', 'theme-sample');
    const heading = node('h4', t('theme.sampleTitle'));
    sample.append(heading, node('p', t('settings.fontPreviewSample')), node('code', 'let theme = "RustRss";'));
    const actions = node('div', '', 'theme-editor-actions');
    const save = button(t('theme.save'), () => commit());
    const cancel = button(t('theme.cancel'), () => reset());
    const preview = button(t('theme.preview'), async () => { await validate(); sample.scrollIntoView({ block: 'nearest' }); });
    actions.append(preview, save, cancel);
    const mode = node('select'); mode.setAttribute('aria-label', t('theme.previewMode'));
    for (const v of ['light', 'dark']) { const o = node('option', t('settings.theme' + (v === 'light' ? 'Light' : 'Dark'))); o.value = v; mode.append(o); }
    mode.onchange = () => { previewMode = mode.value; validate(); };
    host.replaceChildren(node('p', t('theme.draftHint'), 'dim'), form, mode, sample, status, actions);
    const fonts = node('datalist'); fonts.id = host.id + '-fonts'; host.append(fonts);
    function paint(snapshot) {
      renderer ||= global.RustRssTheme.createRenderer(sample, { media: null });
      renderer.apply({ ...snapshot, config: { ...snapshot.config, mode: previewMode } });
      for (const { path, input, type } of controls) {
        if (input === document.activeElement) continue;
        const parts = path.split('.');
        const value = parts[0] === 'colors' ? snapshot[parts[1]].colors[parts[2]] : valueAt(snapshot[previewMode], path);
        if (type === 'boolean') input.checked = value;
        else input.value = Array.isArray(value) ? value.join(', ') : value;
      }
      const warning = snapshot.contrast?.filter(c => !c.passes).length;
      status.textContent = warning ? t('theme.contrastWarning') : t('theme.previewReady');
    }
    async function validate() {
      try { const s = await session.preview(); if (s && !disposed) paint(s); }
      catch (err) { if (!disposed) status.textContent = t('status.settingFailed', { error: err.message }); }
    }
    function change(path, value) {
      session.set(path, value); save.disabled = false; cancel.disabled = false;
      validate();
    }
    function control(field, parent = form) {
      const [path, , type, max, step] = field;
      const row = node('div', '', 'theme-field');
      const label = node('label', t('theme.field.' + path.replace(/^colors\.(light|dark)\./, 'colors.')));
      let input;
      if (Array.isArray(type)) {
        input = node('select');
        for (const value of type) { const opt = node('option', t('theme.option.' + value)); opt.value = value; input.append(opt); }
      } else {
        input = node('input'); input.type = type === 'boolean' ? 'checkbox' : type === 'color' ? 'color' : typeof type === 'number' ? 'number' : 'text';
        if (typeof type === 'number') { input.min = type; input.max = max; input.step = step; }
        if (type === 'font') { input.maxLength = 515; input.placeholder = 'system-ui'; input.setAttribute('list', fonts.id);
          input.onfocus = () => { fonts.replaceChildren(...fontNames().map(name => { const o = node('option'); o.value = name; return o; })); }; }
      }
      input.dataset.themeField = path;
      input.setAttribute('aria-label', label.textContent);
      input.onchange = () => {
        if (!input.checkValidity()) { input.reportValidity(); return; }
        change('overrides.' + path, parseField(field, type === 'boolean' ? input.checked : input.value));
      };
      label.append(input); row.append(label);
      const inherit = button(t('theme.inherit'), () => change('overrides.' + path, null));
      inherit.setAttribute('aria-label', label.firstChild.textContent + ': ' + t('theme.inherit'));
      row.append(inherit); parent.append(row); controls.push({ path, input, type });
    }
    if (kind === 'appearance') {
      const row = node('label', t('settings.theme')); const select = node('select'); select.dataset.themeTop = 'mode';
      for (const v of ['system', 'light', 'dark']) { const opt = node('option', t('settings.theme' + v[0].toUpperCase() + v.slice(1))); opt.value = v; select.append(opt); }
      select.onchange = () => change('mode', select.value); row.append(select); form.append(row);
      for (const variant of ['light', 'dark']) {
        const group = node('div', '', 'theme-presets'); group.append(node('h4', t('theme.' + variant + 'Preset')));
        for (const preset of ['clear', 'paper', 'slate']) {
          const b = button(t('settings.preset.' + preset), () => { change(variant + '_preset', preset); markPresets(); });
          b.dataset.preset = preset; b.dataset.variant = variant; group.append(b);
        }
        form.append(group);
      }
      form.append(button(t('theme.clearOverrides'), () => { session.set('overrides', null); validate(); save.disabled = false; cancel.disabled = false; }));
    }
    fields.filter(f => f[1] === kind).forEach(f => control(f));
    if (kind === 'appearance') {
      for (const variant of ['light', 'dark']) {
        const details = node('details'); details.append(node('summary', t('theme.' + variant + 'Colors')));
        for (const color of Object.keys(global.RustRssTheme.colorVars)) control(['colors.' + variant + '.' + color, kind, 'color'], details);
        form.append(details);
      }
      const history = node('select'); history.setAttribute('aria-label', t('theme.history')); history.dataset.themeHistory = '';
      const restore = button(t('theme.restore'), async () => {
        if (!history.value || busy) return;
        await persist(() => invoke('restore_ui_theme', { expectedRevision: session.base.config.revision, historicalRevision: Number(history.value) }));
      });
      const refreshHistory = async () => {
        try {
          const values = await invoke('get_ui_theme_history'); if (disposed) return;
          history.replaceChildren(); const empty = node('option', t('theme.history')); empty.value = ''; history.append(empty);
          for (const config of values.reverse()) { const opt = node('option', t('theme.historyRevision', { n: config.revision })); opt.value = config.revision; history.append(opt); }
        } catch (err) { status.textContent = err.message; }
      };
      host.append(history, restore, button(t('theme.refreshHistory'), refreshHistory));
      refreshHistory();
    }
    function markPresets() {
      for (const b of form.querySelectorAll('[data-preset]')) b.setAttribute('aria-pressed', String((session.patch[b.dataset.variant + '_preset'] ?? session.base.config[b.dataset.variant + '_preset']) === b.dataset.preset));
    }
    function fill() {
      const base = session.base;
      const modeSelect = form.querySelector('[data-theme-top]'); if (modeSelect) modeSelect.value = base.config.mode;
      markPresets();
      for (const { path, input, type } of controls) {
        const parts = path.split('.');
        const value = parts[0] === 'colors' ? base[parts[1]].colors[parts[2]] : valueAt(base[previewMode], path);
        if (type === 'boolean') input.checked = value;
        else input.value = Array.isArray(value) ? value.join(', ') : value;
      }
      save.disabled = true; cancel.disabled = true; paint(base);
    }
    function reset() { session.reset(getSnapshot()); fill(); }
    async function persist(action) {
      if (busy) return;
      busy = true; form.disabled = true; save.disabled = true; cancel.disabled = true;
      try { const settings = await action(); apply(settings); reset(); status.textContent = t('theme.saved'); }
      catch (err) { status.textContent = t('theme.conflict', { error: err.message }); }
      finally { busy = false; form.disabled = false; save.disabled = !session.dirty; cancel.disabled = !session.dirty; }
    }
    async function commit() {
      if (!session.dirty || !form.reportValidity()) return;
      await persist(() => invoke('update_ui_theme', { expectedRevision: session.base.config.revision, patch: session.patch }));
    }
    fill();
    return { refresh() { if (!session.dirty && !busy) reset(); else if (session.base.config.revision !== getSnapshot().config.revision) status.textContent = t('theme.changedElsewhere'); }, dispose() { disposed = true; session.reset(getSnapshot()); renderer?.dispose(); }, get dirty() { return session.dirty; } };
  }
  global.RustRssThemeSettings = { fields, put, parseField, draftSession, createEditor };
})(typeof window === 'undefined' ? globalThis : window);
