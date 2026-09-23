// Test-only driver: real index.html settings markup + shared production row/reader builders.
(async () => {
  const invoke = window.__TAURI__.core.invoke;
  const report = { fixture_version: 1, captures: [], checks: [], fonts: 'requested/computed stacks only; glyph fallback not inferred' };
  const assert = (ok, name) => { if (!ok) throw new Error(name); report.checks.push(name); };
  const el = id => document.getElementById(id);
  const settle = async () => { await document.fonts.ready; await new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))); };
  try {
    let editor;
    el('pane-appearance').classList.remove('hidden');
    const snapshots = await invoke('snapshots');
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
    const firstRow=el('entries').firstElementChild, firstArticle=el('reader').querySelector('.article');
    assert(I18N.selfTest().ok,'i18n key sets and markup');
    for (const locale of ['zh-CN','en']) {
      I18N.setLocale(locale); I18N.applyStaticI18n();
      for (const [i,snapshot] of snapshots.entries()) for (const mode of ['light','dark']) {
        snapshot.config.mode=mode;
        renderer.apply(snapshot);
        const v=snapshot[mode];
        editor?.dispose();
        editor = RustRssThemeSettings.createEditor(el('appearance-editor'), {kind:'appearance', getSnapshot:()=>snapshot, invoke, apply:()=>{}, t:I18N.t});
        for (const scene of ['overview','article','settings']) {
          el('reader').style.visibility=scene==='overview'?'hidden':'';
          el('settings-overlay').classList.toggle('hidden',scene!=='settings');
          await settle();
          const name=`${['clear','paper','slate'][i]}-${mode}-${locale}-${scene}`;
          report.captures.push({...await invoke('capture_scene',{name}),name,background:v.colors.background,
            ...(scene==='settings'?{settings_background:v.colors.panel,occlusion_point:[Math.round(el('entries').getBoundingClientRect().right-5),150]}:{})});
        }
        assert(el('entries').firstElementChild===firstRow && el('reader').querySelector('.article')===firstArticle,`DOM identity ${i}/${mode}/${locale}`);
      }
    }
    el('settings-overlay').classList.add('hidden');
    el('reader').style.visibility='';
    const s=snapshots[0];s.config.mode='light';renderer.apply(s);await settle();
    const observer=new MutationObserver(()=>{});observer.observe(document.documentElement,{subtree:true,attributes:true,childList:true,characterData:true});
    assert(renderer.apply(s).writes===0 && observer.takeRecords().length===0,'same-value zero DOM mutations');observer.disconnect();
    const target=el('reader').querySelector('[data-paragraph="10"]');
    el('reader').scrollTop+=target.getBoundingClientRect().top-el('reader').getBoundingClientRect().top;
    const before=target.getBoundingClientRect().top;
    s.light.typography.read_size=24;s.light.reader.width=520;
    renderer.apply(s);await settle();
    assert(Math.abs(target.getBoundingClientRect().top-before)<2,'paragraph anchor after font and width change');
    const scroll=el('reader').scrollTop;
    s.light.colors.accent='#884422';renderer.apply(s);await settle();
    assert(el('reader').scrollTop===scroll,'color change preserves scroll');
    s.light.list.summary_lines=0;s.light.list.thumbnail=false;s.light.reader.layout='focus';renderer.apply(s);await settle();
    assert(getComputedStyle(firstRow.querySelector('.summary')).display==='none','zero summary lines');
    assert(getComputedStyle(firstRow.querySelector('.entry-thumbnail')).display==='none','thumbnail hidden without rebuilding rows');
    assert(el('entries').clientWidth>0 && document.querySelector('.sidebar').clientWidth>0,'focus retains navigation');
    report.captures.push(await invoke('capture_scene',{name:'layout-focus'}));
    s.light.typography.read_family=['RustRss Missing Fixture Font','serif'];renderer.apply(s);await settle();
    assert(el('reader').querySelector('.article').getBoundingClientRect().height>100,'missing font retains readable layout via fallback');
    await invoke('narrow');
    for(let i=0;i<100 && innerWidth>950;i++) await new Promise(r=>setTimeout(r,20));
    await settle();
    assert(innerWidth<=950,'narrow viewport acknowledged');
    assert(document.querySelector('.right-col').getBoundingClientRect().right<=innerWidth+1,'narrow reader stays within viewport');
    report.captures.push(await invoke('capture_scene',{name:'narrow-article'}));
    el('settings-overlay').classList.remove('hidden');await settle();
    const button=el('appearance-editor').querySelector('[data-preset]').getBoundingClientRect();
    assert(button.right<innerWidth && button.left>0,'narrow theme picker remains reachable');
    report.captures.push(await invoke('capture_scene',{name:'narrow-settings'}));
    report.viewport=[innerWidth,innerHeight];report.dpr=devicePixelRatio;
  } catch(error) { report.error=String(error.stack||error); }
  await invoke('finish',{report});
})();
