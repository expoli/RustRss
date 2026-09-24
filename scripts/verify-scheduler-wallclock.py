"""Real 15-minute scheduler and disabled-period probe; isolated Xvfb profiles.
No clock injection or shortened product intervals. Requires rebuilt desktop/fixture.
"""
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
spec=importlib.util.spec_from_file_location('probe',Path(__file__).with_name('verify-theme-native-settings.py'))
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
hits=[]
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        hits.append({'path':self.path,'wall':time.time(),'mono':time.monotonic()})
        time.sleep(5)
        body=b"<rss version='2.0'><channel><title>Scheduler fixture</title><item><guid>scheduled-new</guid><title>Scheduled article</title><description>New article</description></item></channel></rss>"
        self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
    def log_message(self,*args):pass

def free_port():
    with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]

def memory(pid):
    total=0;pending=[pid];seen=set()
    while pending:
        p=pending.pop()
        if p in seen:continue
        seen.add(p)
        try:
            for line in Path(f'/proc/{p}/smaps_rollup').read_text().splitlines():
                if line.startswith('Pss:'):total+=int(line.split()[1])
            pending.extend(map(int,Path(f'/proc/{p}/task/{p}/children').read_text().split()))
        except (FileNotFoundError,ProcessLookupError,PermissionError):pass
    return {'pss_kib':total,'pids':sorted(seen)}

async def main():
    subprocess.run(['df','-h','.'],check=True)
    root=Path(tempfile.mkdtemp(prefix='rustrss-scheduler-wallclock-'));print('Evidence:',root,flush=True)
    report={'checks':[],'samples':[],'hits':hits,'profiles':{},'passed':False}
    server=ThreadingHTTPServer(('127.0.0.1',0),Handler);threading.Thread(target=server.serve_forever,daemon=True).start()
    children=[];profiles={};xvfb=None
    def check(condition,label):
        assert condition,label
        report['checks'].append(label)
    async def start(name,initial=False):
        info=profiles[name];log=info['root']/'desktop.log'
        with log.open('w') as out:app=subprocess.Popen(['target/debug/rustrss-desktop'],env=info['env'],stdout=out,stderr=out)
        children.append(app);info['app']=app;info['start']=time.monotonic()
        for _ in range(200):
            if 'loaded feeds=' in log.read_text():break
            assert app.poll() is None,log.read_text()
            await asyncio.sleep(.1)
        await info['probe'].until("!!document.querySelector('#entries li[data-id]')")
    async def invoke(info,command,args):
        await info['probe'].js("window.__schedulerResult=null;window.__TAURI__.core.invoke("+json.dumps(command)+","+json.dumps(args)+").then(value=>window.__schedulerResult={ok:true,value},error=>window.__schedulerResult={ok:false,error:String(error)});true")
        await info['probe'].until('window.__schedulerResult!==null')
        return json.loads(await info['probe'].js('JSON.stringify(window.__schedulerResult)'))
    try:
        read,write=os.pipe();xvfb=subprocess.Popen(['Xvfb','-displayfd',str(write),'-screen','0','1400x1000x24'],pass_fds=(write,),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);os.close(write)
        with os.fdopen(read) as pipe:display=':'+pipe.readline().strip()
        for name,interval in [('enabled','15'),('disabled','off')]:
            folder=root/name;folder.mkdir();dbpath=folder/'fixture.sqlite'
            subprocess.run(['target/debug/examples/theme_fixture',str(dbpath)],check=True)
            seeded=time.time()
            with sqlite3.connect(dbpath) as db:
                db.execute('UPDATE feeds SET url=?,last_fetched_at=?',(f'http://127.0.0.1:{server.server_port}/{name}',int(seeded)))
                db.execute("UPDATE settings SET value=? WHERE key='refresh.interval_minutes'",(interval,))
                db.execute("UPDATE settings SET value='false' WHERE key='refresh.on_start'")
            runtime=folder/'runtime';runtime.mkdir(mode=0o700);port=free_port()
            env=dict(os.environ,HOME=str(folder/'home'),XDG_DATA_HOME=str(folder/'data'),XDG_RUNTIME_DIR=str(runtime),DISPLAY=display,GDK_GL='disable',GDK_SCALE='1',RUSTSS_DB=str(dbpath),RUSTSS_LOG_STDOUT='1',RUSTSS_AI_KEY='isolated-fixture-placeholder',WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{port}')
            for key in ['GDK_BACKEND','WAYLAND_DISPLAY','EGL_PLATFORM']:env.pop(key,None)
            profiles[name]={'root':folder,'db':dbpath,'env':env,'seeded':seeded,'probe':module.Probe({'root':str(folder),'inspector_port':port})}
            report['profiles'][name]={'seeded':seeded,'interval':interval}
            await start(name)
        active=profiles['enabled'];probe=active['probe']
        await probe.js("document.querySelector('#entries li[data-id]').click();true")
        await probe.until("!!document.querySelector('#reader .article')")
        await probe.js("window.__schedulerArticle=document.querySelector('#reader .article');document.getElementById('reader').scrollTop=300;window.__schedulerScroll=document.getElementById('reader').scrollTop;true")
        begun=time.monotonic();last_sample=-60
        while not hits:
            elapsed=time.monotonic()-begun
            assert elapsed<990,'scheduler did not fire within 16.5 minutes'
            assert all(i['app'].poll() is None for i in profiles.values()),'desktop exited during wall-clock wait'
            if elapsed-last_sample>=60:
                last_sample=elapsed
                sample={'elapsed':round(elapsed,2),'memory':{n:memory(i['app'].pid) for n,i in profiles.items()},'disk_free':os.statvfs(root).f_bavail*os.statvfs(root).f_frsize}
                report['samples'].append(sample);(root/'progress.json').write_text(json.dumps(report,indent=2));print('wallclock',round(elapsed),'seconds; hits',len(hits),flush=True)
                subprocess.run(['df','-h','.'],check=True)
                assert sample['disk_free']>5*1024**3,'disk headroom too low'
            await asyncio.sleep(.25)
        check(hits[0]['path']=='/enabled','only enabled profile reaches due source')
        check(hits[0]['wall']-int(active['seeded'])>=900,'real 900-second interval elapsed before scheduled request')
        rejection=await invoke(active,'refresh_all',{'concurrency':1})
        check(not rejection['ok'] and '刷新已在进行中' in rejection.get('error',''),'manual refresh rejected while scheduled request is in flight')
        await probe.until("document.getElementById('status').textContent.includes('Refreshed')||!document.getElementById('status').textContent.includes('Background')")
        await asyncio.sleep(6)
        check(len(hits)==1,'single flight prevents a second HTTP request')
        check(await probe.js("document.querySelector('#reader .article')===window.__schedulerArticle && Math.abs(document.getElementById('reader').scrollTop-window.__schedulerScroll)<2"),'scheduled refresh preserves article node and scroll position')
        with sqlite3.connect(active['db']) as db:check(db.execute('SELECT COUNT(*) FROM entries').fetchone()[0]==31,'scheduled result persisted one new article')
        off_elapsed=time.time()-profiles['disabled']['seeded'];check(off_elapsed>=900 and not any(h['path']=='/disabled' for h in hits),'off profile sends no requests across at least 900 wall-clock seconds')
        report['off_elapsed_seconds']=off_elapsed;report['manual_rejection']=rejection
        # Startup timing is independent of the periodic interval.
        result=await invoke(active,'set_refresh_interval',{'minutes':'off'});check(result['ok'],'disable periodic refresh through real IPC')
        result=await invoke(active,'set_refresh_on_start',{'enabled':True});check(result['ok'],'enable startup refresh through real IPC')
        active['app'].terminate();active['app'].wait(timeout=10);before=len(hits)
        await start('enabled');deadline=time.monotonic()+25
        while len(hits)==before:
            assert time.monotonic()<deadline,'startup refresh missing'
            await asyncio.sleep(.1)
        startup=hits[-1]['mono']-active['start'];report['startup_delay_seconds']=startup
        check(9<=startup<=20,'startup refresh occurs after production ten-second delay')
        await asyncio.sleep(6)
        report['passed']=True;print(json.dumps({'passed':True,'checks':len(report['checks']),'startup_delay':startup}),flush=True)
    finally:
        (root/'results.json').write_text(json.dumps(report,ensure_ascii=False,indent=2))
        for child in children:
            if child.poll() is None:
                child.terminate()
                try:child.wait(timeout=10)
                except subprocess.TimeoutExpired:child.kill();child.wait(timeout=10)
        if xvfb and xvfb.poll() is None:xvfb.terminate();xvfb.wait(timeout=10)
        server.shutdown();server.server_close()

if __name__=='__main__':asyncio.run(main())
