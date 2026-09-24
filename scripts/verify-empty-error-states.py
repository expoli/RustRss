"""Isolated desktop empty/search/fetch-failure checks with a loopback HTTP fixture.
Requires distro Python websockets, Xvfb, xdotool and ImageMagick. No external HTTP.
Run after rebuilding the desktop whenever ui/ changes. Does not alter the session.
"""
import argparse
import asyncio
import importlib.util
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

FEED = b'''<?xml version="1.0"?><rss version="2.0"><channel><title>Local fixture</title><link>http://localhost/</link><description>Fixture</description><item><guid>local-1</guid><title>Recovery article</title><description>Cached body remains readable.</description></item></channel></rss>'''
state = {'mode': 'fail', 'requests': 0}
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        state['requests'] += 1
        mode = state['mode']
        if mode == 'drop':
            self.connection.shutdown(socket.SHUT_RDWR)
            self.connection.close()
            return
        if mode == 'discover_once':
            state['mode'] = 'fail'
            mode = 'ok'
        data = FEED if mode == 'ok' else b'not an RSS document' if mode == 'malformed' else b'unavailable'
        self.send_response(200 if mode in ['ok', 'malformed'] else 503)
        self.send_header('Content-Type','application/rss+xml')
        self.send_header('Content-Length',str(len(data)))
        self.end_headers(); self.wfile.write(data)
    def log_message(self, *args): pass


def port():
    with socket.socket() as s:
        s.bind(('127.0.0.1',0)); return s.getsockname()[1]


async def main(options):
    subprocess.run(['df','-h','.'],check=True)
    root=Path(tempfile.mkdtemp(prefix='rustrss-empty-errors-'))
    print('Evidence:',root,flush=True)
    dbpath=root/'fixture.sqlite'
    subprocess.run(['target/debug/examples/theme_fixture',str(dbpath)],check=True)
    with sqlite3.connect(dbpath) as db:
        db.execute('PRAGMA foreign_keys=ON');db.execute('DELETE FROM entries');db.execute('DELETE FROM feeds')
        for key,value in [('ui.locale',options.locale),('ui.theme',options.theme)]:
            db.execute('UPDATE settings SET value=? WHERE key=?',(value,key))
    inspector=port();runtime=root/'runtime';runtime.mkdir(mode=0o700)
    env=dict(os.environ,HOME=str(root/'home'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),
             RUSTSS_DB=str(dbpath),RUSTSS_LOG_STDOUT='1',RUSTSS_AI_KEY='isolated-fixture-placeholder',
             WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{inspector}',GDK_GL='disable',GDK_SCALE='1')
    for key in ['GDK_BACKEND','WAYLAND_DISPLAY','EGL_PLATFORM']: env.pop(key,None)
    report={'locale':options.locale,'theme':options.theme,'checks':[],'issues':[],'observations':[],'captures':[]}
    app=xvfb=None
    server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    probe=module.Probe({'root':str(root),'inspector_port':inspector})
    def check(ok, text):
        (report['checks'] if ok else report['issues']).append(text)
    def stored():
        with sqlite3.connect(dbpath) as db:
            return {'feeds':db.execute('SELECT COUNT(*) FROM feeds').fetchone()[0],
                    'entries':db.execute('SELECT COUNT(*) FROM entries').fetchone()[0],
                    'status':db.execute('SELECT last_status,last_error FROM feeds').fetchall()}
    async def observe(label):
        data=json.loads(await probe.js("""JSON.stringify({list:document.getElementById('entries').innerText,
          reader:document.getElementById('reader').innerText,status:document.getElementById('status').textContent,
          statusError:document.getElementById('status').classList.contains('error'),
          failed:[...document.querySelectorAll('#feeds .dot')].some(n=>!n.hidden),
          tooltips:[...document.querySelectorAll('#feeds li[data-feed-id]')].map(n=>n.title),
          addDisabled:document.getElementById('add-ok').disabled,refreshDisabled:document.getElementById('btn-refresh').disabled,
          rows:document.querySelectorAll('#entries li[data-id]').length,overflow:document.documentElement.scrollWidth>innerWidth})"""))
        data.update(label=label,database=stored());report['observations'].append(data);return data
    async def click(id): await probe.js(f'document.getElementById({json.dumps(id)}).click();true')
    async def search(query):
        await probe.js("(()=>{const e=document.getElementById('search');e.value="+json.dumps(query)+";e.dispatchEvent(new Event('input',{bubbles:true}));return true;})()")
        await asyncio.sleep(.6)
    async def refresh():
        count=state['requests'];await click('btn-refresh')
        await probe.until("!document.getElementById('btn-refresh').disabled")
        assert state['requests']>count
    def capture(label):
        window=subprocess.check_output(['xdotool','search','--onlyvisible','--name','^RustRss$'],env=env,text=True,timeout=10).splitlines()[0]
        subprocess.run(['xdotool','windowsize',window,'1239','820','windowsize',window,'1240','820'],env=env,check=True,timeout=10)
        time.sleep(.2)
        subprocess.run(['import','-window',window,str(root/(label+'.png'))],env=env,check=True,timeout=10)
        report['captures'].append(label+'.png')
    try:
        r,w=os.pipe();xvfb=subprocess.Popen(['Xvfb','-displayfd',str(w),'-screen','0','1400x1000x24'],pass_fds=(w,),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);os.close(w)
        with os.fdopen(r) as pipe:env['DISPLAY']=':'+pipe.readline().strip()
        with (root/'desktop.log').open('w') as output:app=subprocess.Popen(['target/debug/rustrss-desktop'],env=env,stdout=output,stderr=output)
        for _ in range(150):
            if 'loaded feeds=' in (root/'desktop.log').read_text():break
            assert app.poll() is None
            await asyncio.sleep(.1)
        await probe.until("!!document.querySelector('#entries li')")
        check(await probe.js('I18N.selfTest().ok'),'bilingual keys and markup')
        check(await probe.js('document.documentElement.dataset.theme')==options.theme,'requested theme applied')
        o=await observe('no-subscriptions');capture('no-subscriptions')
        check(o['database']['feeds']==0 and not o['overflow'],'fresh profile empty layout')
        check('Add feed' in o['list'] or 'OPML' in o['list'],'empty subscriptions offer an actionable next step')
        await click('btn-add')
        url=f'http://127.0.0.1:{server.server_port}/feed'
        await probe.js('document.getElementById("add-url").value='+json.dumps(url)+';true')
        await click('add-ok');await probe.until("!document.getElementById('add-ok').disabled")
        o=await observe('discovery-503');check(o['statusError'] and o['database']['feeds']==0,'discovery failure preserves empty database and reports error')
        check(await probe.js('document.getElementById("add-url").value')==url,'discovery failure retains URL for retry')
        state['mode']='discover_once';await click('add-ok');await probe.until("!document.getElementById('add-ok').disabled")
        o=await observe('first-fetch-503');capture('first-fetch-503')
        check(o['database']['feeds']==1 and o['failed'],'first fetch failure retains subscription and shows badge')
        check(o['statusError'] and ('failed' in o['status'].lower() if options.locale=='en' else '失败' in o['status']),'first fetch failure is not reported as successful addition')
        state['mode']='ok';await refresh();o=await observe('recovered')
        check(o['rows']==1 and not o['failed'] and o['database']['entries']==1,'retry recovers article and clears error badge')
        await search('zzzznomatch');o=await observe('empty-search');capture('empty-search')
        check(o['rows']==0 and o['database']['entries']==1,'empty search does not delete cached article')
        check(('match' in o['list'].lower() if options.locale=='en' else '匹配' in o['list']),'empty search explains there are no matching articles')
        await search('Recovery');await probe.until("document.querySelectorAll('#entries li[data-id]').length===1")
        check(True,'changing search query restores matching result')
        await search('');await probe.until("document.querySelectorAll('#entries li[data-id]').length===1")
        for failure in ['fail','malformed','drop']:
            state['mode']=failure;await refresh();o=await observe('refresh-'+failure)
            check(o['failed'] and o['database']['entries']==1 and not o['refreshDisabled'],f'{failure} preserves cached article and enables retry')
        await probe.js("document.querySelector('#entries li[data-id]').click();true")
        await probe.until("!!document.querySelector('#reader .article')")
        check('Cached body' in await probe.js("document.querySelector('#reader .article').textContent"),'cached body readable while connection fails')
        capture('refresh-disconnected')
        state['mode']='ok';await refresh();o=await observe('final-recovery')
        check(not o['failed'] and o['database']['entries']==1,'final retry clears persisted failure without duplicating article')
        report['passed']=not report['issues']
        print(json.dumps(report,ensure_ascii=False),flush=True)
        assert report['passed'],report['issues']
    finally:
        (root/'results.json').write_text(json.dumps(report,ensure_ascii=False,indent=2))
        server.shutdown();server.server_close()
        for child in [app,xvfb]:
            if child and child.poll() is None:
                child.terminate()
                try:child.wait(timeout=10)
                except subprocess.TimeoutExpired:child.kill();child.wait(timeout=10)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--locale',choices=['en','zh-CN'],default='en')
    parser.add_argument('--theme',choices=['light','dark'],default='light')
    asyncio.run(main(parser.parse_args()))
