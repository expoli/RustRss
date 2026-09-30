// Static, local icons shared by navigation, reader actions and menus.
(function (global) {
  'use strict';
  const paths = {
    articles: '<rect x="4" y="3" width="16" height="18" rx="2"/><path d="M8 8h8M8 12h8M8 16h5"/>',
    subscriptions: '<path d="M4 6h16M4 12h16M4 18h16"/><circle cx="7" cy="6" r="1" fill="currentColor" stroke="none"/><circle cx="7" cy="12" r="1" fill="currentColor" stroke="none"/><circle cx="7" cy="18" r="1" fill="currentColor" stroke="none"/>',
    star: '<path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2L12 17.3l-5.6 2.9 1.1-6.2L3 9.6l6.2-.9z"/>',
    later: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3.5 2"/>',
    unread: '<circle cx="12" cy="12" r="8"/><path d="M12 7v5"/>',
    all: '<path d="M5 6h14M5 12h14M5 18h14"/>',
    settings: '<circle cx="12" cy="12" r="3"/><path d="M10 3h4l.5 2.1 1.6.7 1.9-1.1 2.8 2.8-1.1 1.9.7 1.6L22 11v3l-2.1.5-.7 1.6 1.1 1.9-2.8 2.8-1.9-1.1-1.6.7L14 22h-4l-.5-2.1-1.6-.7-1.9 1.1-2.8-2.8 1.1-1.9-.7-1.6L2 14v-3l2.1-.5.7-1.6-1.1-1.9 2.8-2.8 1.9 1.1 1.6-.7z"/>',
    more: '<circle cx="5" cy="12" r="1.5" fill="currentColor" stroke="none"/><circle cx="12" cy="12" r="1.5" fill="currentColor" stroke="none"/><circle cx="19" cy="12" r="1.5" fill="currentColor" stroke="none"/>',
    back: '<path d="m14.5 5-7 7 7 7"/>',
    check: '<path d="m4 12 5 5L20 6"/>',
    sort: '<path d="M7 4v16m0 0-3-3m3 3 3-3M17 20V4m0 0-3 3m3-3 3 3"/>',
    search: '<circle cx="10.8" cy="10.8" r="6.8"/><path d="m16 16 5 5"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    minimize: '<path d="M5 12h14"/>',
    maximize: '<rect x="5" y="5" width="14" height="14" rx="1"/>',
    close: '<path d="M5 5l14 14M19 5 5 19"/>',
    refresh: '<path d="M20 7v5h-5M4 17v-5h5"/><path d="M5.8 9A7 7 0 0 1 18 7l2 5M4 12l2 5a7 7 0 0 0 12.2-2"/>',
    appearance: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2m0 16v2M4.9 4.9l1.4 1.4m11.4 11.4 1.4 1.4M2 12h2m16 0h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>',
    reading: '<path d="M12 6c-3-2-6-2-9-1v14c3-1 6-1 9 1 3-2 6-2 9-1V5c-3-1-6-1-9 1zm0 0v14"/>',
    ai: '<path d="m12 2 1.8 6.2L20 10l-6.2 1.8L12 18l-1.8-6.2L4 10l6.2-1.8zM19 17l.6 1.4L21 19l-1.4.6L19 21l-.6-1.4L17 19l1.4-.6z"/>',
    data: '<rect x="3" y="5" width="18" height="15" rx="2"/><path d="M3 10h18M9 15h6"/>',
    link: '<path d="M10 13a5 5 0 0 0 7.1 0l2-2A5 5 0 0 0 12 3l-1 1M14 11a5 5 0 0 0-7.1 0l-2 2A5 5 0 0 0 12 21l1-1"/>',
  };
  function svg(name) {
    const path = paths[name];
    if (!path) throw new Error(`Unknown icon: ${name}`);
    return `<svg class="ui-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">${path}</svg>`;
  }
  function hydrate(root = document) {
    for (const node of root.querySelectorAll('[data-icon]')) {
      if (!node.querySelector('.ui-icon')) node.insertAdjacentHTML('afterbegin', svg(node.dataset.icon));
    }
  }
  if (typeof document !== 'undefined') hydrate();
  global.RustRssIcons = { svg, hydrate };
})(typeof window === 'undefined' ? globalThis : window);
