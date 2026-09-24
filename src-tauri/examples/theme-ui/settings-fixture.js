// Test-only driver; production editor, renderer and markup with isolated core IPC.
(async () => {
 const invoke=window.__TAURI__.core.invoke, el=id=>document.getElementById(id);
 const report={checks:[],captures:[]};
 const assert=(ok,name)=>{if(!ok)throw Error(name);report.checks.push(name);};
 const wait=async fn=>{for(let i=0;i<100;i++){if(await fn())return;await new Promise(r=>setTimeout(r,30));}throw Error('condition timed out');};
 const tick=()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r)));
 let state=(await invoke('editor_state')).theme_snapshot;
 const renderer=RustRssTheme.createRenderer(document.documentElement,{reader:el('reader'),list:el('entries')});
 const apply=settings=>{state=settings.theme_snapshot;renderer.apply(state);};
 const host=el('appearance-editor'); let editor;
 let fontNames=[];
 const mount=(target,kind)=>RustRssThemeSettings.createEditor(target,{kind,getSnapshot:()=>state,invoke,apply,t:I18N.t,fontNames:()=>fontNames});
 const action=(target,key)=>[...target.querySelectorAll('button')].find(b=>b.textContent===I18N.t(key));
 const change=(target,path,value)=>{const input=target.querySelector(`[data-theme-field="${path}"]`);input.value=value;input.dispatchEvent(new Event('change'));};
 const snap=()=>invoke('editor_state');
 const capture=async name=>{await document.fonts.ready;await tick();report.captures.push(await invoke('capture_scene',{name}));};
 try {
  I18N.setLocale('en');I18N.applyStaticI18n();assert(I18N.selfTest().ok,'bilingual key sets and markup');
  renderer.apply(state);
  // Exercise production rendering with a synthetic OS preference event. This
  // proves the media callback contract, not a compositor-level theme change.
  let mediaChanged;
  const media={matches:false,addEventListener:(_,fn)=>mediaChanged=fn,removeEventListener:()=>{mediaChanged=null;}};
  const systemRenderer=RustRssTheme.createRenderer(document.documentElement,{media});
  const systemSnapshot={...state,config:{...state.config,mode:'system'}};
  const empty=el('reader').querySelector('.reader-empty');
  systemRenderer.apply(systemSnapshot);
  for(const locale of ['en','zh-CN']) {
    I18N.setLocale(locale);I18N.applyStaticI18n();
    for(const dark of [false,true]) {
      media.matches=dark;
      mediaChanged();
      await tick();
      assert(document.documentElement.dataset.theme===(dark?'dark':'light')&&empty.isConnected&&empty.textContent.trim().length>0,
        `empty reader survives system preference ${locale}/${dark}`);
    }
  }
  assert((await snap()).theme_snapshot.config.revision===0,'system preference changes never persist');
  // Restore the real snapshot before handing this root back to its normal renderer.
  systemRenderer.apply(state);
  systemRenderer.dispose();assert(mediaChanged===null,'disposed renderer releases preference listener');
  I18N.setLocale('en');I18N.applyStaticI18n();renderer.apply(state);
  el('reader').innerHTML=RustRssComponents.article(Array.from({length:60},(_,i)=>`<p data-p="${i}">Paragraph ${i}. ${'Typography preserves the paragraph. 中文正文保持阅读位置。'.repeat(8)}</p>`).join(''));
  el('entries').innerHTML='<li data-id="1" class="active">Selected article</li>';
  el('reader').scrollTop=700;
  const article=el('reader').firstElementChild,row=el('entries').firstElementChild;
  const paragraph=[...article.children].find(p=>p.getBoundingClientRect().bottom>el('reader').getBoundingClientRect().top);
  const offset=()=>paragraph.getBoundingClientRect().top-el('reader').getBoundingClientRect().top;
  const before=offset();
  el('settings-overlay').classList.remove('hidden');el('pane-appearance').classList.remove('hidden');
  editor=mount(host,'appearance');
  host.querySelector('[data-preset="paper"][data-variant="light"]').click();
  change(host,'chrome.radius','8');
  const fontInput=host.querySelector('[data-theme-field="typography.ui_family"]');
  fontInput.focus();fontInput.value='Draft family, sans-serif';
  fontNames=['Noto Serif','Noto Sans'];editor.refreshFonts();
  assert(host.querySelector('datalist').options.length===2&&document.activeElement===fontInput&&fontInput.value==='Draft family, sans-serif'&&editor.dirty,
    'late font suggestions preserve focused text and existing draft');
  // Restore the uncommitted text so it cannot become part of the next save.
  fontInput.value=state.light.typography.ui_family.join(', ');fontInput.blur();
  await wait(()=>host.querySelector('.theme-sample').style.getPropertyValue('--radius')==='8px');
  assert((await snap()).theme_snapshot.config.revision===0,'draft preview has zero persistent writes');
  await capture('appearance-draft');
  action(host,'theme.save').click();await wait(()=>state.config.revision===1);
  assert(state.config.light_preset==='paper'&&state.config.dark_preset==='clear','independent light and dark presets');
  assert(state.config.overrides.chrome.radius===8,'sparse override saved');
  assert(article===el('reader').firstElementChild&&row===el('entries').firstElementChild,'save retains article and row nodes');
  assert(Math.abs(offset()-before)<2,'save preserves paragraph offset');
  change(host,'chrome.radius','12');await tick();action(host,'theme.cancel').click();
  assert(!editor.dirty&&(await snap()).theme_snapshot.config.revision===1,'discard does not write');
  action(host,'theme.refreshHistory').click();await wait(()=>host.querySelector('[data-theme-history]').options.length>1);
  host.querySelector('[data-theme-history]').value='0';action(host,'theme.restore').click();await wait(()=>state.config.revision===2);
  assert(state.config.light_preset==='clear','history restore creates a new revision');
  change(host,'chrome.radius','9');
  await invoke('update_ui_theme',{expectedRevision:2,patch:{mode:'dark'}});
  action(host,'theme.save').click();await wait(()=>host.querySelector('.theme-editor-status').textContent.includes('Save failed'));
  assert(editor.dirty&&(await snap()).theme_snapshot.config.revision===3,'stale draft fails CAS and remains editable');
  state=(await snap()).theme_snapshot;action(host,'theme.cancel').click();
  editor.dispose();el('settings-overlay').classList.add('hidden');
  const aa=el('aa-editor');editor=mount(aa,'reading');el('aa-dialog').showModal();
  change(aa,'typography.read_family','Noto Serif, serif');
  change(aa,'typography.read_size','23');change(aa,'reader.width','620');
  await wait(()=>aa.querySelector('.theme-sample').style.getPropertyValue('--font-read-size')==='23px');
  action(aa,'theme.save').click();await wait(()=>state.config.revision===4);
  assert(state.config.overrides.typography.read_size===23&&state.config.overrides.reader.width===620,'Aa saves shared reading fields');
  assert(state.config.overrides.typography.read_family.join(',')==='Noto Serif,serif','font family and generic fallback persist via Aa');
  await capture('aa-reading');
  el('aa-dialog').close();editor.dispose();
  el('settings-overlay').classList.remove('hidden');el('pane-appearance').classList.add('hidden');el('pane-reading').classList.remove('hidden');
  editor=mount(el('reading-editor'),'reading');
  assert(el('reading-editor').querySelector('[data-theme-field="typography.read_size"]').value==='23','reading settings reflect Aa');
  await invoke('narrow');await tick();
  I18N.setLocale('zh-CN');I18N.applyStaticI18n();editor.dispose();editor=mount(el('reading-editor'),'reading');
  assert(document.documentElement.scrollWidth<=innerWidth,'narrow window has no horizontal overflow');
  await capture('reading-narrow-zh');
  // CSS zoom stresses fractional layout only; it is not native compositor scaling.
  for(const zoom of [1.25,1.5]) {
    document.documentElement.style.zoom=String(zoom);await tick();
    const field=el('reading-editor').querySelector('input');
    assert(field.getBoundingClientRect().width>0&&el('reading-editor').scrollWidth<=el('reading-editor').clientWidth+1,
      `reading fields fit at CSS zoom ${zoom}`);
  }
  document.documentElement.style.zoom='';
  report.limitations=['system preference events are synthetic', 'fractional checks use CSS zoom, not native Wayland scaling'];
  const fontList=el('reading-editor').querySelector('datalist');
  const observer=new MutationObserver(()=>{});observer.observe(fontList,{childList:true});
  editor.dispose();editor.refreshFonts();assert(observer.takeRecords().length===0,'disposed editor ignores late font refresh');observer.disconnect();
  report.final_revision=state.config.revision;
 } catch(error){report.error=String(error.stack||error);}
 await invoke('finish',{report});
})();
