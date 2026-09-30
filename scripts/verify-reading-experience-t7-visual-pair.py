"""Capture a same-fixture native Linux 12-variant grid on a selected binary."""
import asyncio
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe = importlib.util.module_from_spec(spec); spec.loader.exec_module(native_probe)

def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]

async def main():
    out = Path(sys.argv[1]).resolve(); out.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix='rustrss-t6-visual-'))
    (root/'home').mkdir(); (root/'runtime').mkdir(mode=0o700)
    db = root/'fixture.sqlite'; shutil.copy2('/tmp/rustrss-t4-fixture.sqlite', db)
    binary = Path(os.environ.get('T7_VISUAL_BINARY', 'target/debug/rustrss-desktop')).resolve()
    baseline = bool(os.environ.get('T7_VISUAL_BASELINE'))
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    env = dict(os.environ, HOME=str(root/'home'), XDG_DATA_HOME=str(root/'data'), XDG_RUNTIME_DIR=str(root/'runtime'), RUSTSS_DB=str(db), GDK_GL='disable', WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{free_port()}')
    env.pop('DISPLAY', None); env.pop('WAYLAND_DISPLAY', None)
    display = next(':'+str(i) for i in range(70, 90) if not Path(f'/tmp/.X11-unix/X{i}').exists())
    xvfb = subprocess.Popen(['/tmp/rustrss-t4-xvfb/root/usr/bin/Xvfb', display, '-screen', '0', '1600x1000x24', '-nolisten', 'tcp', '-ac'], env=env, stdout=(out/'visual-xvfb.log').open('w'), stderr=subprocess.STDOUT)
    app = None
    try:
        for _ in range(200):
            if Path('/tmp/.X11-unix/X'+display[1:]).exists(): break
            assert xvfb.poll() is None
            await asyncio.sleep(.1)
        env.update(DISPLAY=display, GDK_BACKEND='x11')
        app = subprocess.Popen([str(binary)], env=env, stdout=(out/'visual-app.log').open('w'), stderr=subprocess.STDOUT)
        probe = native_probe.Probe({'root':str(root), 'inspector_port':int(env['WEBKIT_INSPECTOR_HTTP_SERVER'].split(':')[-1])})
        for _ in range(200):
            try: await probe.js('1'); break
            except (OSError, IndexError): assert app.poll() is None; await asyncio.sleep(.1)
        await probe.until('!!document.querySelector("#entries li[data-id=\\"1\\"]")')
        listing = subprocess.check_output(['xwininfo', '-root', '-tree'], env=env, text=True)
        window = re.search(r'(0x[0-9a-f]+) "RustRss"', listing).group(1)
        async def invoke(name, args=None):
            await probe.js('window.__T6_RESULT="pending";window.__TAURI__.core.invoke('+json.dumps(name)+','+json.dumps(args or {})+').then(v=>window.__T6_RESULT={ok:true,value:v},e=>window.__T6_RESULT={ok:false,error:String(e)});true')
            await probe.until('window.__T6_RESULT!=="pending"')
            result = await probe.js('window.__T6_RESULT')
            assert result['ok'], result
            return result['value']
        settings = await invoke('get_ui_settings')
        revision = settings['theme_snapshot']['config']['revision']
        overrides = {'typography':{'ui_family':['Noto Sans CJK SC','sans-serif'],'ui_size':16}, 'list':{'density':'compact'}, 'chrome':{'sidebar_width':220,'list_width':320}, 'colors':{'light':{'accent':'#1455a0'}}}
        settings = await invoke('update_ui_theme', {'expectedRevision':revision,'patch':{'overrides':overrides}})
        config = settings['theme_snapshot']['config']
        assert config['overrides'] == overrides, config
        variants = []
        image_hashes = set()
        async def capture_page(page, filename):
            state = await probe.js('''(() => {
              const root=document.documentElement, overlay=document.querySelector('#settings-overlay');
              const reader=document.querySelector('#reader');
              const article=reader?.querySelector('.article');
              const appearance=document.querySelector('#pane-appearance');
              return {locale:root.lang, theme:root.dataset.theme, ready:document.readyState,
                listRows:document.querySelectorAll('#entries li[data-id]').length,
                listTitle:document.querySelector('#list-title')?.textContent?.trim(),
                activeRow:document.querySelector('#entries li.active')?.dataset.id??null,
                readerTitle:reader?.querySelector('h1')?.textContent?.trim()??null,
                readerTextLength:article?.textContent?.trim().length??0,
                settingsOpen:!!overlay&&!overlay.classList.contains('hidden'),
                appearanceVisible:!!appearance?.getClientRects().length,
                settingsTabs:[...document.querySelectorAll('#settings-nav-list [role=tab]')].filter(n=>n.getClientRects().length).length,
                viewport:[innerWidth,innerHeight], scrollWidth:root.scrollWidth};
            })()''')
            assert state['ready']=='complete' and state['listRows']>0 and state['listTitle'], (page,state)
            assert state['locale'].startswith('en' if locale=='en' else 'zh') and state['theme']==mode, (page,state)
            assert state['scrollWidth']<=state['viewport'][0]+1, (page,state)
            if page=='articles':
                assert not state['settingsOpen'] and not state['appearanceVisible'] and state['activeRow'] is None, state
            elif page=='reader':
                assert not state['settingsOpen'] and state['readerTitle'] and state['readerTextLength']>100 and state['activeRow'], state
            else:
                assert state['settingsOpen'] and state['appearanceVisible'] and state['settingsTabs']>=6, state
            await asyncio.sleep(.22) # WebKit layout and native window paint after CDP state change.
            subprocess.run(['import','-display',display,'-window',window,str(out/filename)],env=env,check=True)
            image_hash=hashlib.sha256((out/filename).read_bytes()).hexdigest()
            assert image_hash not in image_hashes, ('stale or repeated native frame',page,filename,image_hash)
            image_hashes.add(image_hash)
            return {'screenshot':filename,'sha256':image_hash,'state':state}
        for locale in ('en','zh-CN'):
            await invoke('set_ui_locale', {'locale':locale})
            for mode in ('light','dark'):
                for preset in ('clear','paper','slate'):
                    current = await invoke('get_ui_settings')
                    revision = current['theme_snapshot']['config']['revision']
                    setting = await invoke('update_ui_theme', {'expectedRevision':revision,'patch':{'mode':mode,'light_preset':preset,'dark_preset':preset}})
                    snapshot = setting['theme_snapshot']
                    assert all(c['ratio']+1e-6 >= c['minimum'] for c in snapshot['contrast']), snapshot['contrast']
                    await probe.js('location.reload();true')
                    await asyncio.sleep(.35)
                    await probe.until('!!document.querySelector("#entries li[data-id]")')
                    prefix=f'visual-{preset}-{mode}-{locale}'
                    articles=await capture_page('articles',prefix+'-articles.png')
                    await probe.js("document.querySelector('#entries li[data-id]').click();true")
                    await probe.until('!!document.querySelector("#reader .article")')
                    await probe.js('document.querySelector("#btn-settings").focus();true')
                    css = await probe.js('''(() => {const root=document.documentElement,s=getComputedStyle(root),settings=document.querySelector('#btn-settings'),r=settings?.getBoundingClientRect();return {locale:document.documentElement.lang,theme:root.dataset.theme,sidebar:s.getPropertyValue('--sidebar-width').trim(),list:s.getPropertyValue('--list-width').trim(),fontSize:s.getPropertyValue('--font-ui-size').trim(),rowPadding:s.getPropertyValue('--row-padding').trim(),accent:s.getPropertyValue('--accent').trim(),bodyScrollWidth:root.scrollWidth,innerWidth,navWidth:document.querySelector('#m-nav')?.getBoundingClientRect().width??null,focused:document.activeElement?.id,focusRule:settings?getComputedStyle(settings).outlineStyle:null,buttonRect:r?{x:r.x,y:r.y,w:r.width,h:r.height}:null}})()''')
                    if not baseline:
                        assert css['theme']==mode and css['sidebar']=='220px' and css['list']=='320px' and css['fontSize']=='16px' and css['rowPadding']=='5px', css
                        assert css['bodyScrollWidth']<=css['innerWidth']+1 and css['navWidth']==0 and css['focused']=='btn-settings', css
                    if mode=='light': assert css['accent']=='#1455a0', css
                    if locale=='en': assert css['locale'].startswith('en'), css
                    else: assert css['locale'].startswith('zh'), css
                    reader=await capture_page('reader',prefix+'-reader.png')
                    await probe.js('document.querySelector("#btn-settings").click();true')
                    await probe.until('!document.querySelector("#settings-overlay").classList.contains("hidden") && !!document.querySelector("#pane-appearance").getClientRects().length')
                    settings_page=await capture_page('settings',prefix+'-settings.png')
                    variants.append({'preset':preset,'mode':mode,'locale':locale,'screenshot':reader['screenshot'],'pages':{'articles':articles,'reader':reader,'settings':settings_page},'css':css,'minContrast':min(c['ratio']-c['minimum'] for c in snapshot['contrast']),'revision':snapshot['config']['revision']})
        assert len(variants)==12 and len(image_hashes)==36, (len(variants),len(image_hashes))
        with sqlite3.connect(db) as conn:
            saved = json.loads(conn.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone()[0])
            saved_locale = conn.execute("SELECT value FROM settings WHERE key='ui.locale'").fetchone()[0]
        assert saved['current']['overrides']==overrides and saved_locale=='zh-CN'
        app.terminate(); app.wait(timeout=10); app=None
        app = subprocess.Popen([str(binary)],env=env,stdout=(out/'visual-restart-app.log').open('w'),stderr=subprocess.STDOUT)
        for _ in range(200):
            try:
                restarted=await probe.js('({theme:document.documentElement.dataset.theme,sidebar:document.documentElement.style.getPropertyValue("--sidebar-width"),locale:document.documentElement.lang})')
                if restarted['sidebar']=='220px': break
            except (OSError,IndexError): pass
            await asyncio.sleep(.1)
        if not baseline: assert restarted['theme']=='dark' and restarted['sidebar']=='220px' and restarted['locale'].startswith('zh'), restarted
        report={'binarySha256':digest,'baseline':baseline,'fixtureSha256':hashlib.sha256(Path('/tmp/rustrss-t4-fixture.sqlite').read_bytes()).hexdigest(),'pageCount':36,'uniqueImageHashes':len(image_hashes),'variants':variants,'overrides':overrides,'restart':restarted,'savedLocale':saved_locale,'passed':True}
        (out/'visual-results.json').write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps({'variants':len(variants),'pages':len(image_hashes),'binarySha256':digest}))
    finally:
        if app and app.poll() is None: app.terminate(); app.wait(timeout=10)
        xvfb.terminate(); xvfb.wait(timeout=10)

if __name__=='__main__': asyncio.run(main())
