"""500-feed/10k-entry production desktop probe; five-minute idle PSS and refresh.
Uses existing debug binary and an isolated database. No global cache dropping.
"""
import asyncio
import ctypes
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
sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location('wallclock',Path(__file__).with_name('verify-scheduler-wallclock.py'))
helper=importlib.util.module_from_spec(spec);spec.loader.exec_module(helper)
lock=threading.Lock();traffic={'active':0,'maximum':0,'requests':[]}
class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        with lock:
            traffic['active']+=1;traffic['maximum']=max(traffic['maximum'],traffic['active']);traffic['requests'].append(self.path)
        try:
            time.sleep(.04)
            body=b"<rss version='2.0'><channel><title>Refreshed</title></channel></rss>"
            self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers();self.wfile.write(body)
        finally:
            with lock:traffic['active']-=1
    def log_message(self,*args):pass

def residency(path):
    # mincore only observes mapping residency; it does not fault in document pages.
    libc=ctypes.CDLL(None,use_errno=True);size=path.stat().st_size
    libc.mmap.restype=ctypes.c_void_p
    descriptor=os.open(path,os.O_RDONLY)
    try:address=libc.mmap(None,ctypes.c_size_t(size),1,1,descriptor,0)
    finally:os.close(descriptor)
    if address==ctypes.c_void_p(-1).value:return None
    pages=(size+4095)//4096;vec=(ctypes.c_ubyte*pages)()
    try:
        result=libc.mincore(ctypes.c_void_p(address),ctypes.c_size_t(size),vec)
        return {'resident_pages':sum(v&1 for v in vec),'pages':pages} if result==0 else None
    finally:libc.munmap(ctypes.c_void_p(address),ctypes.c_size_t(size))

async def main():
    subprocess.run(['df','-h','.'],check=True)
    root=Path(tempfile.mkdtemp(prefix='rustrss-scale-performance-'));print('Evidence:',root,flush=True)
    dbpath=root/'fixture.sqlite';subprocess.run(['target/debug/examples/theme_fixture',str(dbpath)],check=True)
    server=ThreadingHTTPServer(('127.0.0.1',0),Handler);threading.Thread(target=server.serve_forever,daemon=True).start()
    with sqlite3.connect(dbpath) as db:
        template=db.execute('SELECT content_html FROM entries LIMIT 1').fetchone()[0]
        db.execute('DELETE FROM entries');db.execute('DELETE FROM feeds')
        for feed in range(1,501):
            db.execute('INSERT INTO feeds(id,url,title,created_at) VALUES(?,?,?,1)',(feed,f'http://127.0.0.1:{server.server_port}/feed/{feed}',f'Feed {feed:03}'))
        db.executemany('INSERT INTO entries(feed_id,stable_id,id_origin,title,summary,content_html,content_text,search_tokens,content_hash,fetched_at) VALUES(?,?,?,?,?,?,?,?,?,?)',
            [((i//20)+1,f'entry-{i}','source_data',f'Article {i:05}','Fixture summary',template,'Searchable fixture body','searchable fixture body','fixture',1700000000+i) for i in range(10000)])
        db.execute("UPDATE settings SET value='off' WHERE key='refresh.interval_minutes'")
        db.execute("UPDATE settings SET value='false' WHERE key='refresh.on_start'")
        db.commit();db.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()
    with dbpath.open('rb') as file:os.posix_fadvise(file.fileno(),0,0,os.POSIX_FADV_DONTNEED)
    report={'binary':'target/debug/rustrss-desktop','feeds':500,'entries':10000,'database_bytes':dbpath.stat().st_size,'database_cache_before':residency(dbpath),'samples':[],'traffic':traffic,'checks':[],'passed':False}
    runtime=root/'runtime';runtime.mkdir(mode=0o700);port=helper.free_port();app=xvfb=None
    env=dict(os.environ,HOME=str(root/'home'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),GDK_GL='disable',GDK_SCALE='1',RUSTSS_DB=str(dbpath),RUSTSS_LOG_STDOUT='1',RUSTSS_AI_KEY='isolated-fixture-placeholder',WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{port}')
    for key in ['GDK_BACKEND','WAYLAND_DISPLAY','EGL_PLATFORM']:env.pop(key,None)
    probe=helper.module.Probe({'root':str(root),'inspector_port':port})
    def check(value,label):assert value,label;report['checks'].append(label)
    try:
        read,write=os.pipe();xvfb=subprocess.Popen(['Xvfb','-displayfd',str(write),'-screen','0','1400x1000x24'],pass_fds=(write,),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);os.close(write)
        with os.fdopen(read) as pipe:env['DISPLAY']=':'+pipe.readline().strip()
        start=time.monotonic()
        with (root/'desktop.log').open('w') as out:app=subprocess.Popen(['target/debug/rustrss-desktop'],env=env,stdout=out,stderr=out)
        for _ in range(300):
            if 'loaded feeds=' in (root/'desktop.log').read_text():break
            assert app.poll() is None
            await asyncio.sleep(.02)
        await probe.until("!!document.querySelector('#entries li[data-id]')")
        report['startup_interactive_seconds']=time.monotonic()-start
        check(await probe.js("document.querySelectorAll('#feeds li[data-feed-id]').length") == 500,'all 500 feeds rendered')
        print('startup',report['startup_interactive_seconds'],'seconds',flush=True)
        idle=time.monotonic()
        for index in range(6):
            if index:await asyncio.sleep(max(0,idle+60*index-time.monotonic()))
            sample={'idle_seconds':time.monotonic()-idle,'memory':helper.memory(app.pid),'disk_free':os.statvfs(root).f_bavail*os.statvfs(root).f_frsize}
            report['samples'].append(sample);(root/'progress.json').write_text(json.dumps(report,indent=2));print('idle',round(sample['idle_seconds']),'PSS KiB',sample['memory']['pss_kib'],flush=True)
            subprocess.run(['df','-h','.'],check=True);assert sample['disk_free']>5*1024**3
        check(not traffic['requests'],'no spontaneous feed requests during idle')
        await probe.js("window.__scaleFrames=[];window.__scaleMeasuring=true;let last=performance.now();function tick(now){window.__scaleFrames.push(now-last);last=now;if(window.__scaleMeasuring)requestAnimationFrame(tick);}requestAnimationFrame(tick);document.getElementById('btn-refresh').click();true")
        started=time.monotonic();queries=[]
        while await probe.js("document.getElementById('btn-refresh').disabled"):
            assert time.monotonic()-started<120,'refresh exceeded two minutes'
            q=time.monotonic();await probe.js("document.getElementById('entries').scrollTop+=80;document.querySelector('#entries li[data-id]')?.click();true");queries.append((time.monotonic()-q)*1000)
            await asyncio.sleep(.1)
        report['refresh_seconds']=time.monotonic()-started;report['interaction_roundtrip_ms']=queries
        await probe.js('window.__scaleMeasuring=false;true')
        report['frame_intervals_ms']=json.loads(await probe.js('JSON.stringify(window.__scaleFrames)'))
        check(len(traffic['requests'])==500 and len(set(traffic['requests']))==500,'one HTTP request for each of 500 feeds')
        check(traffic['maximum']<=6,'default concurrency remains at most six')
        with sqlite3.connect(dbpath) as db:check(db.execute('SELECT COUNT(*) FROM entries').fetchone()[0]==10000,'refresh keeps all cached entries')
        report['passed']=True;print(json.dumps({'passed':True,'checks':len(report['checks']),'max_concurrency':traffic['maximum'],'refresh_seconds':report['refresh_seconds']}),flush=True)
    finally:
        (root/'results.json').write_text(json.dumps(report,ensure_ascii=False,indent=2))
        for child in [app,xvfb]:
            if child and child.poll() is None:
                child.terminate()
                try:child.wait(timeout=10)
                except subprocess.TimeoutExpired:child.kill();child.wait(timeout=10)
        server.shutdown();server.server_close()
if __name__=='__main__':asyncio.run(main())
