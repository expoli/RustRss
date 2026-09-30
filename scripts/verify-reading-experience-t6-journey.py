"""Same-process Linux WebKitGTK feed-to-reader-to-settings journey with local RSS."""
import asyncio
import ctypes
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
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import xml.etree.ElementTree as ET

sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location('native_probe',Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe=importlib.util.module_from_spec(spec);spec.loader.exec_module(native_probe)

def port():
    with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]

class Feed(BaseHTTPRequestHandler):
    def do_GET(self):
        body=b'''<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>T6 Local Reading</title><link>http://127.0.0.1/</link><description>Isolated fixture</description><item><guid>t6-one</guid><title>T6 Converged Article</title><link>http://127.0.0.1/article</link><description>native local journey</description><pubDate>Wed, 30 Sep 2026 12:00:00 GMT</pubDate></item></channel></rss>'''
        self.send_response(200);self.send_header('Content-Type','application/rss+xml; charset=utf-8');self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
    def log_message(self,*args):pass

async def main():
    out=Path(sys.argv[1]).resolve();out.mkdir(parents=True,exist_ok=True)
    root=Path(tempfile.mkdtemp(prefix='rustrss-t6-journey-'));(root/'home').mkdir();(root/'runtime').mkdir(mode=0o700)
    db=root/'fixture.sqlite';shutil.copy2('/tmp/rustrss-t4-fixture.sqlite',db)
    with sqlite3.connect(db) as conn:conn.execute("INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES('ui.locale','en',1790740800)")
    feedserver=ThreadingHTTPServer(('127.0.0.1',0),Feed);threading.Thread(target=feedserver.serve_forever,daemon=True).start()
    env=dict(os.environ,HOME=str(root/'home'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(root/'runtime'),RUSTSS_DB=str(db),GDK_GL='disable',WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{port()}')
    env.pop('DISPLAY',None);env.pop('WAYLAND_DISPLAY',None)
    display=next(':'+str(i) for i in range(70,90) if not Path(f'/tmp/.X11-unix/X{i}').exists())
    xvfb=subprocess.Popen(['/tmp/rustrss-t4-xvfb/root/usr/bin/Xvfb',display,'-screen','0','1600x1000x24','-nolisten','tcp','-ac'],env=env,stdout=(out/'journey-xvfb.log').open('w'),stderr=subprocess.STDOUT)
    app=None;checks=[]
    def check(name,ok,detail=None):
        assert ok,f'{name}: {detail}';checks.append({'name':name,'detail':detail})
    try:
        for _ in range(200):
            if Path('/tmp/.X11-unix/X'+display[1:]).exists():break
            assert xvfb.poll() is None;await asyncio.sleep(.1)
        env.update(DISPLAY=display,GDK_BACKEND='x11')
        app=subprocess.Popen(['target/debug/rustrss-desktop'],env=env,stdout=(out/'journey-app.log').open('w'),stderr=subprocess.STDOUT)
        probe=native_probe.Probe({'root':str(root),'inspector_port':int(env['WEBKIT_INSPECTOR_HTTP_SERVER'].split(':')[-1])})
        for _ in range(200):
            try:await probe.js('1');break
            except (OSError,IndexError):assert app.poll() is None;await asyncio.sleep(.1)
        await probe.until('!!document.querySelector("#entries li[data-id=\\"1\\"]")')
        listing=subprocess.check_output(['xwininfo','-root','-tree'],env=env,text=True);found=re.search(r'(0x[0-9a-f]+) "RustRss"',listing);assert found
        window=found.group(1)
        subprocess.run(['import','-display',display,'-window',window,str(out/'journey-start.png')],env=env,check=True)
        await probe.js('document.querySelector("#btn-add").click();true')
        check('add-form-open',await probe.js('!document.querySelector("#add-row").classList.contains("hidden")'))
        url=f'http://127.0.0.1:{feedserver.server_port}/feed.xml'
        await probe.js(f'''(()=>{{const n=document.querySelector('#add-url');n.value={json.dumps(url)};document.querySelector('#add-ok').click();return true}})()''')
        await probe.until('!![...document.querySelectorAll("#feeds li[data-feed-id]")].find(n=>n.textContent.includes("T6 Local Reading"))')
        with sqlite3.connect(db) as conn:
            added=conn.execute('SELECT id,title,folder_id FROM feeds WHERE url=?',(url,)).fetchone()
            entry=conn.execute('SELECT id,title,starred,read_later FROM entries WHERE feed_id=?',(added[0],)).fetchone()
        check('add-feed-and-fetch',bool(added and entry and entry[1]=='T6 Converged Article'),{'feed':added,'entry':entry})
        feed_id,entry_id=added[0],entry[0]
        await probe.js('document.querySelector("#btn-new-folder").click();true')
        await probe.until('!!document.querySelector(".prompt-overlay input")')
        await probe.js('document.querySelector(".prompt-overlay input").value="T6 Reading Group";document.querySelector(".prompt-overlay [data-act=ok]").click();true')
        await probe.until('!![...document.querySelectorAll("#feeds li[data-folder-id]")].find(n=>n.textContent.includes("T6 Reading Group"))')
        with sqlite3.connect(db) as conn: folder_id=conn.execute("SELECT id FROM folders WHERE name='T6 Reading Group'").fetchone()[0]
        check('folder-created',folder_id>0,folder_id)
        await probe.js(f'''document.querySelector('#feeds li[data-feed-id="{feed_id}"] .row-more').click();true''')
        await probe.until('!!document.querySelector("#ctx-menu")')
        await probe.js('''(()=>{const n=[...document.querySelectorAll('#ctx-menu > button')].find(x=>x.textContent.includes('Move to'));n.focus();n.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true,cancelable:true}));return true})()''')
        await probe.until('!!document.querySelector(".ctx-submenu")')
        await probe.js('''[...document.querySelectorAll('.ctx-submenu button')].find(n=>n.textContent.includes('T6 Reading Group')).click();true''')
        for _ in range(50):
            with sqlite3.connect(db) as conn:moved=conn.execute('SELECT folder_id FROM feeds WHERE id=?',(feed_id,)).fetchone()[0]
            if moved==folder_id:break
            await asyncio.sleep(.1)
        check('feed-moved-via-menu',moved==folder_id,moved)
        await probe.js('document.querySelector("#btn-list-sort").click();true')
        await probe.until('!!document.querySelector("#ctx-menu")')
        await probe.js('''[...document.querySelectorAll('#ctx-menu > button')].find(n=>n.textContent.includes('Oldest')).click();true''')
        await probe.until('!document.querySelector("#ctx-menu")')
        with sqlite3.connect(db) as conn:sort=conn.execute("SELECT value FROM settings WHERE key='list.sort'").fetchone()
        check('sort-oldest-saved',sort and sort[0]=='oldest',sort)
        await probe.js('document.querySelector("#btn-search-toggle").click();document.querySelector("#search").value="T6 Converged";document.querySelector("#search").dispatchEvent(new Event("input",{bubbles:true}));true')
        await probe.until(f'document.body.dataset.listKind==="search" && !!document.querySelector("#entries li[data-id=\\"{entry_id}\\"]")')
        check('search-finds-added-article',await probe.js('document.body.dataset.listKind==="search"'))
        await probe.js(f'document.querySelector("#entries li[data-id=\\"{entry_id}\\"] .title").click();true')
        await probe.until('!!document.querySelector("#reader .article")')
        check('reader-opened',await probe.js('document.querySelector("#reader").textContent.includes("T6 Converged Article")'))
        subprocess.run(['import','-display',display,'-window',window,str(out/'journey-reader.png')],env=env,check=True)
        await probe.js('document.querySelector("#act-star").click();document.querySelector("#act-later").click();true')
        for _ in range(50):
            with sqlite3.connect(db) as conn:flags=conn.execute('SELECT starred,read_later FROM entries WHERE id=?',(entry_id,)).fetchone()
            if flags==(1,1):break
            await asyncio.sleep(.1)
        check('star-later-independent',flags==(1,1),flags)
        await probe.js('document.querySelector("#reader .tag-add").click();true')
        await probe.until('!document.querySelector("#tag-picker-overlay").classList.contains("hidden")')
        await probe.js('''(()=>{const n=document.querySelector('#tag-picker-input');n.value='T6 Topic';n.dispatchEvent(new Event('input',{bubbles:true}));return true})()''')
        await probe.js('document.querySelector("#tag-picker-list li.create").click();true')
        for _ in range(50):
            with sqlite3.connect(db) as conn:tag=conn.execute("SELECT t.id FROM tags t JOIN entry_tags et ON et.tag_id=t.id WHERE t.name='T6 Topic' AND et.entry_id=?",(entry_id,)).fetchone()
            if tag:break
            await asyncio.sleep(.1)
        check('tag-attached',bool(tag),tag)
        await probe.js('document.querySelector("#tag-picker-close").click();document.querySelector("#btn-search-cancel").click();true')
        await probe.until('document.body.dataset.listKind!=="search"')
        check('return-from-search-reader',await probe.js('document.querySelector("#reader .article")===null && !document.querySelector("#ctx-menu")'))
        await probe.js('document.querySelector("#views li[data-kind=all]").click();true')
        await probe.until('!!document.querySelector("#entries li[data-id=\\"1\\"]")')
        await probe.js('document.querySelector("#entries li[data-id=\\"1\\"] .title").click();true')
        await probe.until('document.querySelector("#reader .article")?.textContent.length>500')
        await probe.js('document.querySelector("#reader").scrollTop=420;true')
        before_anchor=await probe.js('''(() => {const r=document.querySelector('#reader'),p=[...r.querySelectorAll('.article p')].find(n=>n.getBoundingClientRect().bottom>r.getBoundingClientRect().top+8);window.__T6_ANCHOR=p;return {top:p.getBoundingClientRect().top,scroll:r.scrollTop,entry:document.querySelector('#entries li.active')?.dataset.id}})()''')
        await probe.js('document.querySelector("#btn-settings").click();document.querySelector("#tab-reading").click();document.querySelector("#reading-editor .theme-advanced").open=true;true')
        await probe.js('''(()=>{const w=document.querySelector('#reading-editor [data-theme-field="reader.width"]'),l=document.querySelector('#reading-editor [data-theme-field="reader.layout"]');w.value='760';w.dispatchEvent(new Event('change',{bubbles:true}));l.value='focus';l.dispatchEvent(new Event('change',{bubbles:true}));return true})()''')
        await probe.js('document.querySelector("#reading-editor .theme-editor-actions button:nth-child(2)").click();true')
        await probe.until('document.documentElement.dataset.readerLayout==="focus"')
        focus_anchor=await probe.js('''({top:window.__T6_ANCHOR.getBoundingClientRect().top,scroll:document.querySelector('#reader').scrollTop,same:window.__T6_ANCHOR.isConnected,entry:document.querySelector('#entries li.active')?.dataset.id,width:document.documentElement.style.getPropertyValue('--reader-width'),grid:getComputedStyle(document.querySelector('main')).gridTemplateColumns})''')
        check('focus-layout-width-anchor',focus_anchor['same'] and focus_anchor['entry']==before_anchor['entry'] and focus_anchor['width']=='760px' and abs(focus_anchor['top']-before_anchor['top'])<3,{'before':before_anchor,'after':focus_anchor})
        subprocess.run(['import','-display',display,'-window',window,str(out/'journey-focus-layout.png')],env=env,check=True)
        await probe.js('''(()=>{const w=document.querySelector('#reading-editor [data-theme-field="reader.width"]');w.value='800';w.dispatchEvent(new Event('change',{bubbles:true}));document.querySelector('#reading-editor .theme-editor-actions button:nth-child(3)').click();return true})()''')
        check('reading-cancel-restores-saved-width',await probe.js('document.querySelector("#reading-editor [data-theme-field=\\"reader.width\\"]").value==="760"'))
        await probe.js('document.querySelector("#tab-appearance").click();document.querySelector("#appearance-editor .theme-advanced").open=true;true')
        await probe.js('''(()=>{const n=document.querySelector('#appearance-editor [data-theme-field="typography.ui_size"]');n.value='17';n.dispatchEvent(new Event('change',{bubbles:true}));return true})()''')
        with sqlite3.connect(db) as conn:before_theme=conn.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone()[0]
        await probe.js('document.querySelector("#appearance-editor .theme-editor-actions button:first-child").click();true')
        await probe.until('document.querySelector("#appearance-editor .theme-editor-status").textContent.length>0')
        with sqlite3.connect(db) as conn:preview_theme=conn.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone()[0]
        check('appearance-preview-zero-write',preview_theme==before_theme)
        await probe.js('document.querySelector("#appearance-editor .theme-editor-actions button:nth-child(2)").click();true')
        for _ in range(50):
            with sqlite3.connect(db) as conn:saved_theme=conn.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone()[0]
            if saved_theme!=before_theme:break
            await asyncio.sleep(.1)
        check('appearance-save-persists',saved_theme!=before_theme and json.loads(saved_theme)['current']['overrides']['typography']['ui_size']==17)
        await probe.js('''(()=>{const n=document.querySelector('#appearance-editor [data-theme-field="typography.ui_size"]');n.value='18';n.dispatchEvent(new Event('change',{bubbles:true}));document.querySelector('#appearance-editor .theme-editor-actions button:nth-child(3)').click();return true})()''')
        with sqlite3.connect(db) as conn:cancel_theme=conn.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone()[0]
        check('appearance-cancel-zero-write',cancel_theme==saved_theme and await probe.js('document.querySelector("#appearance-editor [data-theme-field=\\"typography.ui_size\\"]").value==="17"'))
        await probe.js('document.querySelector("#settings-close").click();true')
        check('theme-return-keeps-reader',await probe.js('window.__T6_ANCHOR.isConnected && !!document.querySelector("#entries li.active")'))
        await probe.js('document.querySelector("#btn-settings").click();document.querySelector("#tab-data").click();document.querySelector("#act-export-opml").click();true')
        await asyncio.sleep(.8)
        windows=subprocess.check_output(['xwininfo','-root','-tree'],env=env,text=True)
        (out/'journey-opml-windows.txt').write_text(windows.rstrip()+'\n')
        picker=re.search(r'(0x[0-9a-f]+) "Save File"',windows);assert picker,windows
        subprocess.run(['import','-display',display,'-window',picker.group(1),str(out/'journey-opml-picker.png')],env=env,check=True)
        export_path=root/'home'/'t6-export.opml'
        subprocess.run(['xsel','-ib'],input=export_path.name,text=True,env=env,check=True)
        x11=ctypes.CDLL('libX11.so.6');xtst=ctypes.CDLL('libXtst.so.6')
        x11.XOpenDisplay.argtypes=[ctypes.c_char_p];x11.XOpenDisplay.restype=ctypes.c_void_p
        x11.XStringToKeysym.argtypes=[ctypes.c_char_p];x11.XStringToKeysym.restype=ctypes.c_ulong
        x11.XKeysymToKeycode.argtypes=[ctypes.c_void_p,ctypes.c_ulong];x11.XKeysymToKeycode.restype=ctypes.c_ubyte
        x11.XSetInputFocus.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.c_int,ctypes.c_ulong]
        x11.XSync.argtypes=[ctypes.c_void_p,ctypes.c_int]
        xtst.XTestFakeKeyEvent.argtypes=[ctypes.c_void_p,ctypes.c_uint,ctypes.c_int,ctypes.c_ulong]
        xtst.XTestFakeMotionEvent.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.c_int,ctypes.c_int,ctypes.c_ulong]
        xtst.XTestFakeButtonEvent.argtypes=[ctypes.c_void_p,ctypes.c_uint,ctypes.c_int,ctypes.c_ulong]
        handle=x11.XOpenDisplay(display.encode());assert handle
        native_window=int(picker.group(1),16)
        def key(name,ctrl=False):
            x11.XSetInputFocus(handle,native_window,2,0)
            names=(['Control_L'] if ctrl else [])+[name]
            codes=[x11.XKeysymToKeycode(handle,x11.XStringToKeysym(n.encode())) for n in names]
            assert all(codes),names
            for code in codes:xtst.XTestFakeKeyEvent(handle,code,1,0)
            for code in reversed(codes):xtst.XTestFakeKeyEvent(handle,code,0,0)
            x11.XSync(handle,0)
        def click(x,y):
            xtst.XTestFakeMotionEvent(handle,-1,x,y,0)
            xtst.XTestFakeButtonEvent(handle,1,1,0);xtst.XTestFakeButtonEvent(handle,1,0,0);x11.XSync(handle,0)
        click(60,76);await asyncio.sleep(.3)
        click(270,27);key('a',ctrl=True);key('v',ctrl=True);await asyncio.sleep(.2)
        subprocess.run(['import','-display',display,'-window',picker.group(1),str(out/'journey-opml-path.png')],env=env,check=True)
        click(1048,799)
        for _ in range(30):
            if export_path.exists():break
            await asyncio.sleep(.1)
        check('native-opml-export',export_path.exists() and b'T6 Local Reading' in export_path.read_bytes(),{'path':str(export_path),'bytes':export_path.stat().st_size if export_path.exists() else None})
        xml=ET.parse(export_path).getroot()
        nested=xml.find("./body/outline[@text='T6 Reading Group']/outline[@type='rss']")
        check('opml-group-source-roundtrip',nested is not None and nested.attrib.get('xmlUrl')==url,dict(nested.attrib) if nested is not None else None)
        shutil.copy2(export_path,out/'journey-export.opml.xml')
        report={'binarySha256':hashlib.sha256(Path('target/debug/rustrss-desktop').read_bytes()).hexdigest(),'appPid':app.pid,'feedUrl':url,'input':'WebKit inspector DOM controls plus XTest native GTK chooser pointer/keyboard','checks':checks,'passed':True}
        (out/'journey-results.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
        print(json.dumps({'checks':len(checks),'binarySha256':report['binarySha256']}))
    finally:
        if app and app.poll() is None:app.terminate();app.wait(timeout=10)
        xvfb.terminate();xvfb.wait(timeout=10);feedserver.shutdown()

if __name__=='__main__':asyncio.run(main())
