"""Isolated X11 keyboard/help, 10k search and unavailable-feed cached reading.
Requires a rebuilt desktop, theme_fixture, Xvfb, xdotool and Python websockets.
No changes to host networking; unavailable loopback feeds simulate fetch failure.
"""
import asyncio
import argparse
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('helper', Path(__file__).with_name('verify-scheduler-wallclock.py'))
helper = importlib.util.module_from_spec(spec); spec.loader.exec_module(helper)

async def main(offline=False):
    subprocess.run(['df', '-h', '.'], check=True)
    links = None
    if offline:
        links = json.loads(subprocess.check_output(['ip','-json','link'],text=True,timeout=10))
        assert [link['ifname'] for link in links] == ['lo'], 'offline mode requires a private namespace with only loopback'
    root = Path(tempfile.mkdtemp(prefix='rustrss-keyboard-search-'))
    print('Evidence:', root, flush=True)
    dbpath = root/'fixture.sqlite'
    subprocess.run(['target/debug/examples/theme_fixture', str(dbpath)], check=True)
    with sqlite3.connect(dbpath) as db:
        db.execute('DELETE FROM entries')
        db.execute('UPDATE feeds SET url=?', ('http://192.0.2.1/feed' if offline else 'http://127.0.0.1:1/unavailable',))
        db.execute("UPDATE settings SET value='off' WHERE key='refresh.interval_minutes'")
        db.execute("UPDATE settings SET value='false' WHERE key='refresh.on_start'")
        db.execute("INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES('network.proxy',?,1)", (json.dumps({'mode':'direct','url':'','no_proxy':''}),))
        feed = db.execute('SELECT id FROM feeds LIMIT 1').fetchone()[0]
        db.executemany('INSERT INTO entries(feed_id,stable_id,id_origin,title,summary,content_html,content_text,search_tokens,content_hash,fetched_at) VALUES(?,?,?,?,?,?,?,?,?,?)',
            [(feed,f'key-{i}','source_data',f'Common Article {i}', 'Cached summary',
              '<p>Cached offline body common 中文 新闻 '+('uniqueneedle' if i==5000 else 'ordinary')+'</p>',
              'Cached offline body common 中文 新闻 '+('uniqueneedle' if i==5000 else 'ordinary'),
              'common 中文 新闻 '+('uniqueneedle' if i==5000 else 'ordinary'), 'fixture',1700000000+i) for i in range(10000)])
    runtime=root/'runtime';runtime.mkdir(mode=0o700);port=helper.free_port()
    env=dict(os.environ, HOME=str(root/'home'), XDG_DATA_HOME=str(root/'data'), XDG_RUNTIME_DIR=str(runtime),
        GDK_GL='disable',GDK_SCALE='1',RUSTSS_DB=str(dbpath),RUSTSS_LOG_STDOUT='1',RUSTSS_AI_KEY='fixture-placeholder',WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{port}')
    for key in ['GDK_BACKEND','WAYLAND_DISPLAY','EGL_PLATFORM']:env.pop(key,None)
    probe=helper.module.Probe({'root':str(root),'inspector_port':port})
    report={'checks':[],'search_ms':[],'passed':False,'offline_namespace':links is not None};app=xvfb=None
    def check(ok,label):assert ok,label;report['checks'].append(label)
    def key(value):subprocess.run(['xdotool','key','--clearmodifiers',value],env=env,check=True,timeout=10)
    try:
        read,write=os.pipe();xvfb=subprocess.Popen(['Xvfb','-displayfd',str(write),'-screen','0','1400x1000x24'],pass_fds=(write,),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);os.close(write)
        with os.fdopen(read) as pipe:env['DISPLAY']=':'+pipe.readline().strip()
        log=root/'desktop.log'
        with log.open('w') as out:app=subprocess.Popen(['target/debug/rustrss-desktop'],env=env,stdout=out,stderr=out)
        for _ in range(200):
            if 'loaded feeds=' in log.read_text():break
            assert app.poll() is None
            await asyncio.sleep(.1)
        await probe.until("document.querySelectorAll('#entries li[data-id]').length>0")
        window=subprocess.check_output(['xdotool','search','--onlyvisible','--name','^RustRss$'],env=env,text=True,timeout=10).splitlines()[0]
        subprocess.run(['xdotool','windowfocus',window],env=env,check=True,timeout=10)
        for _ in range(40):
            key('Tab')
            if await probe.js("document.activeElement.dataset.kind==='all'"):break
        check(await probe.js("document.activeElement.dataset.kind==='all'"),'native Tab reaches All view')
        key('space');await probe.until("!!document.querySelector('#views li[data-kind=all].active')")
        check(True,'native Space switches sidebar view')
        # Native key injection, not DOM dispatch. The close button receives focus.
        key('question');await probe.until("document.getElementById('keyboard-help').open")
        check(await probe.js("document.activeElement.id==='keyboard-help-close'"),'help opens with focus on close button')
        before=await probe.js("document.querySelector('#entries li.active')?.dataset.id||null")
        key('j');check(await probe.js("document.querySelector('#entries li.active')?.dataset.id||null")==before,'help prevents background navigation')
        key('Escape');await probe.until("!document.getElementById('keyboard-help').open")
        key('j');key('Return');await probe.until("document.getElementById('reader').textContent.includes('Cached offline body')")
        check(True,'native navigation opens cached article without a working feed')
        key('slash');await probe.until("document.activeElement.id==='search'")
        subprocess.run(['xdotool','type','--clearmodifiers','--delay','20','uniqueneedle'],env=env,check=True,timeout=10)
        await probe.until("document.querySelectorAll('#entries li[data-id]').length===1")
        check(await probe.js("document.getElementById('entries').textContent.includes('Article 5000')"),'native search finds body token in 10k entries')
        key('Escape');await probe.until("document.activeElement.id!=='search' && document.querySelectorAll('#entries li[data-id]').length>1")
        await probe.js("(()=>{window.__searchUiMeasure={start:null,done:null};const input=document.getElementById('search');input.addEventListener('input',()=>{if(input.value==='common')window.__searchUiMeasure.start=performance.now()},true);new MutationObserver(()=>{if(window.__searchUiMeasure.start!==null&&document.querySelectorAll('#entries li[data-id]').length===200)window.__searchUiMeasure.done=performance.now()-window.__searchUiMeasure.start}).observe(document.getElementById('entries'),{childList:true,subtree:true});return true})()")
        key('slash');await probe.until("document.activeElement.id==='search'")
        subprocess.run(['xdotool','type','--clearmodifiers','--delay','0','common'],env=env,check=True,timeout=10)
        await probe.until("window.__searchUiMeasure.done!==null")
        report['ui_search_ms']=json.loads(await probe.js('JSON.stringify(window.__searchUiMeasure.done)'))
        check(await probe.js("document.querySelectorAll('#entries li[data-id]').length===200"),'native broad search renders the capped page from 10k matches')
        for _ in range(5):
            await probe.js("(()=>{window.__searchResult=null;const began=performance.now();window.__TAURI__.core.invoke('search',{query:'common',limit:200}).then(rows=>window.__searchResult={ms:performance.now()-began,count:rows.length}).catch(e=>window.__searchResult={error:String(e)});return true;})()")
            await probe.until('window.__searchResult!==null')
            result=json.loads(await probe.js('JSON.stringify(window.__searchResult)'))
            check(result.get('count')==200,'production search IPC returns the capped broad result page')
            report['search_ms'].append(result['ms'])
        key('Escape');await probe.until("document.activeElement.id!=='search' && document.querySelectorAll('#entries li[data-id]').length>1")
        check(True,'Escape leaves search and restores unread list')
        if offline:
            key('j');await probe.until("document.getElementById('reader').textContent.includes('Cached offline body')")
            key('r')
            status = None
            for _ in range(100):
                with sqlite3.connect(dbpath) as db:status=db.execute('SELECT last_status FROM feeds LIMIT 1').fetchone()[0]
                if status == 'connection_error':break
                await asyncio.sleep(.1)
            check(status=='connection_error','refresh fails with connection_error in loopback-only network namespace')
            await probe.until("!document.getElementById('btn-refresh').disabled")
            before=await probe.js("document.querySelector('#entries li.active')?.dataset.id")
            key('j')
            await probe.until("document.querySelector('#entries li.active')?.dataset.id!=="+json.dumps(before))
            check(await probe.js("document.getElementById('reader').textContent.includes('Cached offline body') && !document.querySelector('dialog[open]')"),'cached navigation continues after offline refresh without a modal dialog')
            with sqlite3.connect(dbpath) as db:check(db.execute('SELECT COUNT(*) FROM entries').fetchone()[0]==10000,'offline failure preserves all cached entries')
        report['passed']=True;print(json.dumps(report),flush=True)
    finally:
        (root/'results.json').write_text(json.dumps(report,ensure_ascii=False,indent=2))
        for child in [app,xvfb]:
            if child and child.poll() is None:child.terminate();child.wait(timeout=10)
if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--offline',action='store_true')
    asyncio.run(main(parser.parse_args().offline))
