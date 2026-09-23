// Shared by the desktop and isolated production-component fixtures.
(function (global) {
  'use strict';
  const colorVars = {
    background: '--bg', sidebar: '--bg-sidebar', panel: '--bg-panel', text: '--fg', muted: '--fg-dim',
    accent: '--accent', selected: '--bg-active', hover: '--bg-hover', border: '--line', focus: '--focus',
    danger: '--danger', star: '--star', code_background: '--code-bg', code_text: '--code-fg',
    code_keyword: '--code-kw', code_string: '--code-str', code_number: '--code-num', code_comment: '--code-com',
    code_function: '--code-fn', code_type: '--code-type', code_variable: '--code-var',
    diff_add_background: '--diff-add-bg', diff_add_text: '--diff-add-fg',
    diff_delete_background: '--diff-del-bg', diff_delete_text: '--diff-del-fg', diff_hunk_background: '--diff-hunk-bg',
  };
  const generics = new Set(['serif', 'sans-serif', 'monospace', 'system-ui', 'cursive', 'fantasy']);
  function fontFamily(names) {
    return names.map(name => generics.has(name) ? name : '"' + name.replace(/\\/g, '\\\\').replace(/"/g, '\\"') + '"').join(', ');
  }
  function tokens(values) {
    const result = {};
    for (const [key, variable] of Object.entries(colorVars)) result[variable] = values.colors[key];
    const t = values.typography, r = values.reader, c = values.chrome;
    Object.assign(result, {
      '--font-ui': fontFamily(t.ui_family), '--font-read': fontFamily(t.read_family), '--font-mono': fontFamily(t.mono_family),
      '--font-ui-size': t.ui_size + 'px', '--font-read-size': t.read_size + 'px', '--font-mono-size': t.mono_size + 'px',
      '--font-read-line': String(t.line_height), '--reader-width': r.width + 'px', '--paragraph-gap': r.paragraph_gap + 'em',
      '--radius': c.radius + 'px', '--sidebar-width': c.sidebar_width + 'px', '--list-width': c.list_width + 'px',
      '--summary-lines': String(values.list.summary_lines), '--row-padding': values.list.density === 'compact' ? '5px' : '9px',
      '--diff-add-edge': values.colors.diff_add_text, '--diff-del-edge': values.colors.diff_delete_text,
    });
    return result;
  }
  function anchor(container, selector) {
    if (!container || !container.clientHeight) return null;
    const top = container.getBoundingClientRect().top;
    const node = [...container.querySelectorAll(selector)].find(n => n.getBoundingClientRect().bottom > top);
    return node ? { container, node, offset: node.getBoundingClientRect().top - top } : null;
  }
  function restore(a) {
    if (a && a.node.isConnected) a.container.scrollTop += a.node.getBoundingClientRect().top - a.container.getBoundingClientRect().top - a.offset;
  }
  function createRenderer(root, { reader = null, list = null, media = global.matchMedia?.('(prefers-color-scheme: dark)'), onModeChange = () => {} } = {}) {
    let snapshot = null, generation = 0, layoutKey = '', lastVars = {}, lastAttrs = {}, activePreview = {};
    function apply(next, preview = {}) {
      if (snapshot && next.config.revision < snapshot.config.revision) return { stale: true, writes: 0 };
      snapshot = next;
      activePreview = preview;
      const mode = next.config.mode === 'system' ? (media?.matches ? 'dark' : 'light') : next.config.mode;
      const values = next[mode];
      const vars = tokens(values);
      if (preview.font_read_size != null) vars['--font-read-size'] = preview.font_read_size + 'px';
      if (preview.font_read_line != null) vars['--font-read-line'] = String(preview.font_read_line);
      vars['color-scheme'] = mode;
      const attrs = { theme: mode, density: values.list.density, summaryLines: String(values.list.summary_lines),
        thumbnails: String(values.list.thumbnail), readerLayout: values.reader.layout };
      const geometry = JSON.stringify([values.typography, values.list, values.reader, values.chrome, preview]);
      const layoutChanged = geometry !== layoutKey;
      const anchors = layoutChanged ? [anchor(reader, '.article > *'), anchor(list, 'li[data-id]')] : [];
      let writes = 0;
      for (const [key, value] of Object.entries(vars)) if (lastVars[key] !== value) { root.style.setProperty(key, value); writes++; }
      for (const [key, value] of Object.entries(attrs)) if (lastAttrs[key] !== value) { root.dataset[key] = value; writes++; }
      lastVars = vars; lastAttrs = attrs; layoutKey = geometry;
      if (writes) {
        const current = ++generation;
        anchors.forEach(restore);
        const scrolls = anchors.map(a => a?.container.scrollTop);
        // A late font load may reflow. Never pull the user back after they scroll
        // or after an article/list has been replaced or a newer theme applied.
        Promise.resolve(root.ownerDocument?.fonts?.ready).then(() => {
          if (current !== generation) return;
          anchors.forEach((a, i) => { if (a && a.container.scrollTop === scrolls[i]) restore(a); });
        });
      }
      return { writes, mode, revision: next.config.revision, config_hash: next.config_hash };
    }
    const changed = () => { if (snapshot?.config.mode === 'system') { apply(snapshot, activePreview); onModeChange(); } };
    media?.addEventListener('change', changed);
    return { apply, dispose: () => { generation++; media?.removeEventListener('change', changed); } };
  }
  global.RustRssTheme = { tokens, fontFamily, createRenderer, colorVars };
})(typeof window === 'undefined' ? globalThis : window);
