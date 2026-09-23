// Fixed local content only. No subscription reads, remote images, or production handlers.
(() => {
  const invoke=window.__TAURI__.core.invoke;
  const el=id=>document.getElementById(id);
    const renderer = RustRssTheme.createRenderer(document.documentElement,{reader:el('reader'),list:el('entries')});
    const components = RustRssComponents;
    for (const [icon,label,count] of [['●','Unread · 未读',30],['★','Starred · 星标',3],['⚑','Read later · 稍后读',5]]) {
      const li=document.createElement('li');li.innerHTML=components.viewContent(icon);
      li.querySelector('.vlabel').textContent=label;li.querySelector('.count').textContent=count;
      el('views').append(li);
    }
    for (const [name,failed] of [['Design notes · 设计',false],['Unavailable feed · 错误状态',true]]) {
      const li=document.createElement('li');li.innerHTML=components.feedContent();
      li.querySelector('.name').textContent=name;li.querySelector('.dot').hidden=!failed;li.querySelector('.count').textContent='15';
      el('feeds').append(li);
    }
    el('list-title').textContent = 'Theme fixture · 示例';
    el('list-count').textContent = '30';
    el('entries').innerHTML = Array.from({length:30},(_,i)=>`<li data-id="${i}" class="${i===0?'active':''}">${components.entryContent({
      title: `Reading thoughtfully · 阅读与排版 ${i+1}`, meta:'<span>Local fixture</span><span>09:41</span><span class="star">★</span>',
      summary:'A long bilingual summary to check line wrapping, density and contrast. 中文摘要用来检查文字换行和阅读密度。',
      thumbnail:i===0?'<img class="entry-thumbnail" alt="" src="data:image/svg+xml,%3Csvg xmlns=\'http://www.w3.org/2000/svg\' width=\'48\' height=\'48\'%3E%3Crect width=\'48\' height=\'48\' fill=\'%23875020\'/%3E%3C/svg%3E">':''
    })}</li>`).join('');
    const body = '<p>Local, fixed fixture. 本地固定示例，不读取订阅数据。</p><h2>Readable typography · 可读排版</h2><p><a href="#">A sample link</a> and <code>inline_code</code>.</p><blockquote>Space gives words room to breathe. 留白让文字更容易阅读。</blockquote><pre><code class="language-rust">fn main() { println!("Hello, 世界"); }</code></pre><table><tr><th>Theme</th><th>Reading</th></tr><tr><td>Paper</td><td>Serif</td></tr></table><pre><code class="language-diff">@@ example @@\n- old preference\n+ shared theme</code></pre>' + Array.from({length:40},(_,i)=>`<p data-paragraph="${i}">Paragraph ${i}. 阅读中的位置应该保持稳定。 Changing typography should preserve the current paragraph, selected list row and DOM identity. ${'Text for wrapping. '.repeat(8)}</p>`).join('');
    const article = components.readerHead({title:'A calmer place to read · 安静地阅读',meta:'<span>Local fixture</span><span>2026-09-23</span>'}) + components.article(body);
    el('reader').innerHTML = article;
    document.querySelectorAll('pre code').forEach(node=>hljs.highlightElement(node));

  I18N.setLocale('zh-CN'); I18N.applyStaticI18n();
  const style=document.createElement('style');
  style.textContent='*{animation:none!important;transition:none!important;caret-color:transparent!important} #preview-marker{position:fixed;top:0;left:0;z-index:99999;height:4px;display:flex;pointer-events:none} #preview-marker i{display:block;width:4px;height:4px} #preview-cancel{position:fixed;right:12px;bottom:8px;z-index:99999;background:var(--bg);color:var(--text);border:1px solid var(--border);padding:6px 12px}';
  document.head.append(style);
  const marker=document.createElement('div'); marker.id='preview-marker';document.body.append(marker);
  const cancel=document.createElement('button');cancel.id='preview-cancel';cancel.textContent=I18N.t('themePreview.cancel');
  cancel.onclick=()=>invoke('preview_cancel');document.body.append(cancel);
  document.addEventListener('click',e=>{if(e.target.closest('a'))e.preventDefault();});
  let last='',generation=0;
  async function render(force=false) {
    const request=await invoke('preview_request');
    if(!request || (request.request_id===last && !force))return;
    last=request.request_id;const own=++generation;
    const snapshot=structuredClone(request.snapshot);snapshot.config.mode=request.mode;
    renderer.apply(snapshot);
    el('reader').style.visibility=request.scene==='overview'?'hidden':'';
    el('settings-overlay').classList.toggle('hidden',request.scene!=='settings');
    const values=snapshot[request.mode];
    el('set-theme').textContent=I18N.t('settings.theme'+(request.mode==='dark'?'Dark':'Light'));
    el('set-theme-preset').textContent=I18N.t('settings.preset.'+snapshot.config[request.mode+'_preset']);
    for(const id of ['set-font-ui','set-font-read','set-font-mono'])el(id).textContent=I18N.t('settings.fontFollowSystem');
    el('set-font-size').value=values.typography.read_size;el('set-font-size-value').textContent=values.typography.read_size+'px';
    el('set-font-line').value=values.typography.line_height;el('set-font-line-value').textContent=values.typography.line_height;
    marker.replaceChildren(...request.marker.map(color=>{const cell=document.createElement('i');cell.style.background=color;return cell;}));
    await document.fonts.ready;
    await Promise.all([...document.images].map(img=>img.decode().catch(()=>{})));
    await new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)));
    if(own!==generation)return;
    await invoke('preview_ready',{ready:{request_id:request.request_id,preview_revision:request.preview_revision,config_hash:request.snapshot.config_hash,scene:request.scene,mode:request.mode,viewport:[innerWidth,innerHeight],dpr:devicePixelRatio,fonts:{requested:values.typography,computed:getComputedStyle(el('reader').querySelector('.article')).fontFamily,glyph_fallback:'unknown'}}});
  }
  window.addEventListener('resize',()=>render(true).catch(console.error));
  window.addEventListener('theme-preview-request',()=>render().catch(console.error));
  render().catch(console.error);
})();
