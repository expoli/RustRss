// Pure production markup builders reused by the isolated theme fixture.
(function (global) {
  'use strict';
  function entryContent({ title, meta, summary, thumbnail = '' }) {
    return `${thumbnail}<span class="title">${title}</span><span class="meta">${meta}</span>${summary ? `<span class="summary">${summary}</span>` : ''}`;
  }
  function thumbnailImage(url, escapeHtml) {
    if (!url) return '';
    return `<img class="entry-thumbnail" alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" src="${escapeHtml(url)}">`;
  }
  function readerHead({ title, meta }) {
    return `<div class="reader-head"><h1>${title}</h1><div class="meta">${meta}</div></div>`;
  }
  function article(body) { return `<div class="article">${body}</div>`; }
  function viewContent(icon) { return `<span class="icon">${icon}</span><span class="vlabel"></span><span class="count"></span>`; }
  function feedContent() { return '<span class="name"></span><span class="dot" hidden>●</span><span class="count"></span>'; }
  global.RustRssComponents = { entryContent, thumbnailImage, readerHead, article, viewContent, feedContent };
})(typeof window === 'undefined' ? globalThis : window);
