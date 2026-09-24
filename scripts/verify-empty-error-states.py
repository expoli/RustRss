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
import ssl
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

FEED = b'''<?xml version="1.0"?><rss version="2.0"><channel><title>Local fixture</title><link>http://localhost/</link><description>Fixture</description><item><guid>local-1</guid><title>Recovery article</title><description>Cached body remains readable.</description></item></channel></rss>'''
state = {'mode': 'fail', 'requests': 0, 'proxy_requests': 0, 'stop': threading.Event()}
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        state['proxy_requests' if getattr(self.server, 'is_proxy', False) else 'requests'] += 1
        mode = state['mode']
        if mode == 'slow':
            self.send_response(200);self.send_header('Content-Length','1000');self.end_headers()
            self.wfile.write(b'x');self.wfile.flush()
            state['stop'].wait(32)
            return
        if mode == 'rate':
            self.send_response(429);self.send_header('Retry-After','120');self.send_header('Content-Length','0');self.end_headers()
            return
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
    app=xvfb=tls_server=proxy_server=None
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
    async def refresh(expect_http=True):
        count=state['requests'];await click('btn-refresh')
        deadline=time.monotonic()+45
        while await probe.js("document.getElementById('btn-refresh').disabled"):
            assert time.monotonic()<deadline,'refresh exceeded production deadline'
            await asyncio.sleep(.2)
        if expect_http: assert state['requests']>count
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
        if options.extended:
            state['mode']='rate';before=state['requests'];await refresh();o=await observe('rate-429')
            check(o['database']['status'][0][0]=='http_429' and state['requests']==before+1,'429 retains status and does not immediately retry')
            check(any(('limiting requests' in t if options.locale=='en' else '频率' in t) for t in o['tooltips']),'rate limit hint is localized')
            before=state['requests'];await refresh(expect_http=False)
            check(state['requests']==before,'retry deadline prevents manual network request')
            # Advance only this isolated fixture's deadline; do not sleep two minutes.
            with sqlite3.connect(dbpath) as db:db.execute('UPDATE feeds SET retry_after_at=0')
            state['mode']='slow';started=time.monotonic();await refresh();elapsed=time.monotonic()-started
            o=await observe('body-timeout');o['elapsed_seconds']=round(elapsed,2)
            check(28<=elapsed<45 and o['database']['status'][0][0]=='timeout','real production 30s body deadline is timeout, not HTTP 200')
            check(o['database']['entries']==1 and not o['refreshDisabled'],'timeout preserves cache and releases refresh button')
            cert,key=root/'cert.pem',root/'key.pem'
            subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-keyout',str(key),'-out',str(cert),
                '-subj','/CN=localhost','-addext','subjectAltName=IP:127.0.0.1','-days','1'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,check=True,timeout=15)
            key.chmod(0o600)
            context=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);context.load_cert_chain(cert,key)
            tls_server=ThreadingHTTPServer(('127.0.0.1',0),Handler)
            tls_server.socket=context.wrap_socket(tls_server.socket,server_side=True)
            threading.Thread(target=tls_server.serve_forever,daemon=True).start()
            tls_url=f'https://127.0.0.1:{tls_server.server_port}/feed'
            # Positive control: the same TLS fixture serves valid RSS when a
            # separate test client explicitly trusts only its temporary certificate.
            # The RustRss client never receives that trust override.
            state['mode']='ok'
            with urllib.request.urlopen(tls_url,context=ssl.create_default_context(cafile=str(cert)),timeout=5) as response:
                check(response.read()==FEED,'TLS fixture works with explicit test-only certificate trust')
            with sqlite3.connect(dbpath) as db:db.execute('UPDATE feeds SET url=?',(tls_url,))
            before=state['requests'];await refresh(expect_http=False);o=await observe('untrusted-tls')
            check(o['database']['status'][0][0]=='connection_error' and state['requests']==before,'untrusted certificate rejected before HTTP, no verification bypass')
            check(o['database']['entries']==1,'TLS failure preserves cached article')
            await probe.js('document.getElementById("add-url").value='+json.dumps(tls_url)+';true')
            await click('add-ok');await probe.until("!document.getElementById('add-ok').disabled")
            o=await observe('discovery-untrusted-tls')
            check(o['statusError'] and ('certificate' in o['status'] if options.locale=='en' else '证书' in o['status']),'structured discovery error is localized')
            if options.locale=='en':
                check(not any('\u4e00'<=c<='\u9fff' for o in report['observations'] for t in [o['status'],*o['tooltips']] for c in t),'known English error surfaces contain no Chinese diagnostics')
            with sqlite3.connect(dbpath) as db:db.execute('UPDATE feeds SET url=?',(url,))
        state['mode']='ok';await refresh();o=await observe('final-recovery')
        check(not o['failed'] and o['database']['entries']==1,'final retry clears persisted failure without duplicating article')
        if options.proxy:
            proxy_server=ThreadingHTTPServer(('127.0.0.1',0),Handler);proxy_server.is_proxy=True
            threading.Thread(target=proxy_server.serve_forever,daemon=True).start()
            proxy_url=f'http://127.0.0.1:{proxy_server.server_port}'
            async def proxy_save(mode,address='',bypass=''):
                await probe.js("document.getElementById('set-proxy-mode').value="+json.dumps(mode)+";document.getElementById('set-proxy-mode').dispatchEvent(new Event('change'));document.getElementById('set-proxy-url').value="+json.dumps(address)+";document.getElementById('set-proxy-bypass').value="+json.dumps(bypass)+";true")
                await click('set-proxy-save');await probe.until("!document.getElementById('set-proxy-save').disabled")
            def proxy_stored():
                with sqlite3.connect(dbpath) as db:
                    row=db.execute("SELECT value FROM settings WHERE key='network.proxy'").fetchone()
                    return json.loads(row[0]) if row else None
            await click('btn-settings');await click('tab-subscriptions')
            await proxy_save('custom',proxy_url)
            check(proxy_stored()=={'mode':'custom','url':proxy_url,'no_proxy':''},'proxy UI save persists exact config')
            capture('proxy-settings')
            saved=proxy_stored();await proxy_save('custom','http://user:secret@127.0.0.1:8080')
            check(proxy_stored()==saved,'credential URL rejected without overwriting config')
            await proxy_save('custom',proxy_url)
            await click('settings-close')
            before=state['proxy_requests'];direct=state['requests'];await refresh(expect_http=False)
            check(state['proxy_requests']==before+1 and state['requests']==direct,'saved custom proxy routes desktop refresh')
            await click('btn-settings');await click('tab-subscriptions');await proxy_save('custom',proxy_url,'127.0.0.1');await click('settings-close')
            before=state['proxy_requests'];await refresh()
            check(state['proxy_requests']==before,'custom bypass reaches origin directly')
            await click('btn-settings');await click('tab-subscriptions');await proxy_save('direct');await click('settings-close')
            before=state['proxy_requests'];await refresh()
            check(state['proxy_requests']==before and proxy_stored()['mode']=='direct','direct mode applies immediately')
            app.terminate();app.wait(timeout=10)
            with (root/'desktop.log').open('w') as output:app=subprocess.Popen(['target/debug/rustrss-desktop'],env=env,stdout=output,stderr=output)
            for _ in range(150):
                if 'loaded feeds=' in (root/'desktop.log').read_text():break
                assert app.poll() is None
                await asyncio.sleep(.1)
            await click('btn-settings');await click('tab-subscriptions')
            await probe.until("document.getElementById('set-proxy-mode').value==='direct'")
            check(proxy_stored()['mode']=='direct','proxy mode survives desktop restart')
            await proxy_save('environment');await click('settings-close');await refresh()
            check(proxy_stored()['mode']=='environment','environment mode can be restored')
        if options.organize:
            # Seed only this isolated database, then restart to load real sidebar rows.
            with sqlite3.connect(dbpath) as db:
                db.execute("UPDATE feeds SET custom_title='A fixture'")
                for suffix,title in [('b','B fixture'),('c','C fixture')]:
                    db.execute("INSERT INTO feeds(url,title,created_at) VALUES(?,?,1)",(url+'/'+suffix,title))
            async def restart():
                nonlocal app
                app.terminate();app.wait(timeout=10)
                with (root/'desktop.log').open('w') as output:app=subprocess.Popen(['target/debug/rustrss-desktop'],env=env,stdout=output,stderr=output)
                for _ in range(150):
                    if 'loaded feeds=' in (root/'desktop.log').read_text():break
                    assert app.poll() is None
                    await asyncio.sleep(.1)
                await probe.until("document.querySelectorAll('#feeds li[data-feed-id]').length>0")
            await restart()
            async def order():return json.loads(await probe.js("JSON.stringify([...document.querySelectorAll('#feeds li[data-feed-id]')].map(n=>Number(n.dataset.feedId)))"))
            original=await order();check(len(original)==3,'three real subscription rows loaded')
            # Real X11 pointer drag, not direct command invocation or synthetic drop.
            positions=json.loads(await probe.js("JSON.stringify([...document.querySelectorAll('#feeds li[data-feed-id]')].map(n=>{const r=n.getBoundingClientRect();return {x:r.left+60,y:r.top+r.height/2,top:r.top};}))"))
            window=subprocess.check_output(['xdotool','search','--onlyvisible','--name','^RustRss$'],env=env,text=True,timeout=10).splitlines()[0]
            await probe.js("window.__feedDrag=[];for(const type of ['mousedown','dragstart','dragover','drop','dragend','mouseup'])document.addEventListener(type,e=>{if(window.__feedDrag.length<40)window.__feedDrag.push({type:e.type,x:e.clientX,y:e.clientY,target:e.target.closest('li')?.dataset.feedId||e.target.tagName,accepted:e.defaultPrevented,effect:e.dataTransfer?.dropEffect});},false);true")
            source,target=positions[-1],positions[0]
            geometry=subprocess.check_output(['xdotool','getwindowgeometry','--shell',window],env=env,text=True,timeout=10)
            xy=dict(line.split('=',1) for line in geometry.splitlines() if '=' in line)
            x=int(xy['X'])+round(source['x']);start_y=int(xy['Y'])+round(source['y']);end_y=int(xy['Y'])+round(target['top']+8)
            subprocess.run(['xdotool','windowraise',window,'windowfocus',window,'mousemove',str(x),str(start_y),'mousedown','1'],env=env,check=True,timeout=10)
            await asyncio.sleep(.2)
            for y in range(start_y-5,end_y-1,-5):
                subprocess.run(['xdotool','mousemove',str(x),str(y)],env=env,check=True,timeout=10)
                await asyncio.sleep(.06)
            subprocess.run(['xdotool','mousemove',str(x),str(end_y)],env=env,check=True,timeout=10)
            await asyncio.sleep(.8)
            report['pointer_before_drop']=subprocess.check_output(['xdotool','getmouselocation','--shell'],env=env,text=True,timeout=10)
            report['element_before_drop']=await probe.js("document.elementFromPoint("+str(round(target['x']))+","+str(round(target['top']+8))+").outerHTML")
            subprocess.run(['xdotool','mouseup','1'],env=env,check=True,timeout=10)
            await asyncio.sleep(.3)
            report['drag_observation']={'positions':positions,'events':json.loads(await probe.js('JSON.stringify(window.__feedDrag)')),'order_after':await order()}
            expected=[original[-1],*original[:-1]]
            await probe.until("JSON.stringify([...document.querySelectorAll('#feeds li[data-feed-id]')].map(n=>Number(n.dataset.feedId)))==="+json.dumps(json.dumps(expected,separators=(',',':'))))
            check(await order()==expected,'native pointer drag persists sidebar order')
            await restart();check(await order()==expected,'subscription order survives desktop restart')
            capture('subscription-order')
            async def unsubscribe_menu():
                await probe.js("document.querySelector('#feeds li[data-feed-id]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:100,clientY:220}));true")
                await probe.js("[...document.querySelectorAll('#ctx-menu button')].find(n=>n.textContent.includes(I18N.t('menu.unsubscribe'))).click();true")
                await probe.until("!document.getElementById('generic-confirm-overlay').classList.contains('hidden')")
            await unsubscribe_menu();await click('generic-confirm-cancel')
            check(stored()['feeds']==3 and await order()==expected,'cancel unsubscribe preserves database and sidebar')
            await unsubscribe_menu();await click('generic-confirm-ok')
            await probe.until("document.querySelectorAll('#feeds li[data-feed-id]').length===2")
            check(stored()['feeds']==2 and await order()==expected[1:],'confirm unsubscribe deletes only selected feed')
            # Stored hostile content exercises the actual article rendering path.
            hostile = """<p>Safe reader marker</p><script>window.__rssInjected=1</script>
              <img src="data:image/png;base64,broken" onerror="window.__rssInjected=2">
              <a href="javascript:window.__rssInjected=3">Hostile link</a>
              <svg onload="window.__rssInjected=4"></svg>
              <iframe srcdoc="<script>parent.__rssInjected=5</script>"></iframe>"""
            with sqlite3.connect(dbpath) as db:
                db.execute('UPDATE entries SET content_html=?,content_text=?,read=0',(hostile,'Safe reader marker'))
            await restart()
            await probe.until("!!document.querySelector('#entries li[data-id]')")
            await probe.js("window.__rssInjected=0;window.__positiveControl=0;const testImage=document.createElement('img');testImage.onerror=()=>{window.__positiveControl++;testImage.remove();};testImage.src='data:image/png;base64,broken';document.body.appendChild(testImage);true")
            await probe.until('window.__positiveControl===1')
            await probe.js("document.querySelector('#entries li[data-id]').click();true")
            await probe.until("!!document.querySelector('#reader .article')")
            await asyncio.sleep(.4)
            await probe.js("[...document.querySelectorAll('#reader a')].find(n=>n.textContent==='Hostile link')?.click();true")
            await asyncio.sleep(.2)
            security=json.loads(await probe.js("JSON.stringify({executed:window.__rssInjected,control:window.__positiveControl,text:document.querySelector('#reader .article').textContent,active:[...document.querySelectorAll('#reader .article *')].some(n=>['SCRIPT','IFRAME','SVG'].includes(n.tagName)||[...n.attributes].some(a=>/^on/i.test(a.name)||/^javascript:/i.test(a.value)))})"))
            report['reader_security']=security
            check(security['control']==1 and security['executed']==0 and not security['active'] and 'Safe reader marker' in security['text'],'hostile stored article cannot execute scripts, event handlers or javascript links')

        report['passed']=not report['issues']
        print(json.dumps(report,ensure_ascii=False),flush=True)
        assert report['passed'],report['issues']
    finally:
        (root/'results.json').write_text(json.dumps(report,ensure_ascii=False,indent=2))
        state['stop'].set()
        if tls_server: tls_server.shutdown();tls_server.server_close()
        if proxy_server: proxy_server.shutdown();proxy_server.server_close()
        server.shutdown();server.server_close()
        for child in [app,xvfb]:
            if child and child.poll() is None:
                child.terminate()
                try:child.wait(timeout=10)
                except subprocess.TimeoutExpired:child.kill();child.wait(timeout=10)


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--organize',action='store_true',help='verify native subscription drag, restart and delete confirmation')
    parser.add_argument('--proxy',action='store_true',help='also verify proxy settings, routing and restart')
    parser.add_argument('--extended',action='store_true',help='also test real 30s timeout, 429 and untrusted TLS')
    parser.add_argument('--locale',choices=['en','zh-CN'],default='en')
    parser.add_argument('--theme',choices=['light','dark'],default='light')
    asyncio.run(main(parser.parse_args()))
