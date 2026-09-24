"""Cold database pages on the real disk, plus a warm restart; debug production UI.
Input must be a disposable scale fixture. Leaves the one cache DB for explicit cleanup.
"""
import asyncio
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time
sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location('scale',Path(__file__).with_name('verify-scale-performance.py'))
scale=importlib.util.module_from_spec(spec);spec.loader.exec_module(scale)
async def main(source):
    source=Path(source).resolve();assert source.parent.name.startswith('rustrss-scale-performance-') and str(source).startswith('/tmp/')
    subprocess.run(['df','-h','.'],check=True)
    root=Path(tempfile.mkdtemp(prefix='rustrss-cold-start-'))
    descriptor,name=tempfile.mkstemp(prefix='rustrss-cold-',suffix='.sqlite',dir=Path.home()/'.cache');os.close(descriptor);dbpath=Path(name)
    print('Evidence:',root,'Disk database:',dbpath,flush=True)
    src=sqlite3.connect(source);dest=sqlite3.connect(dbpath)
    src.backup(dest);dest.execute("UPDATE settings SET value='off' WHERE key='refresh.interval_minutes'");dest.execute("UPDATE settings SET value='false' WHERE key='refresh.on_start'");dest.commit();dest.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone();dest.close();src.close()
    with dbpath.open('rb') as file:os.fsync(file.fileno());os.posix_fadvise(file.fileno(),0,0,os.POSIX_FADV_DONTNEED)
    report={'database':str(dbpath),'binary':'target/debug/rustrss-desktop','runs':[],'passed':False}
    cache=scale.residency(dbpath);report['cold_residency']=cache
    assert cache and cache['resident_pages']==0,cache
    runtime=root/'runtime';runtime.mkdir(mode=0o700);port=scale.helper.free_port();app=xvfb=None
    env=dict(os.environ,HOME=str(root/'home'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(runtime),GDK_GL='disable',GDK_SCALE='1',RUSTSS_DB=str(dbpath),RUSTSS_LOG_STDOUT='1',RUSTSS_AI_KEY='isolated-fixture-placeholder',WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{port}')
    for key in ['GDK_BACKEND','WAYLAND_DISPLAY','EGL_PLATFORM']:env.pop(key,None)
    probe=scale.helper.module.Probe({'root':str(root),'inspector_port':port})
    try:
        read,write=os.pipe();xvfb=subprocess.Popen(['Xvfb','-displayfd',str(write),'-screen','0','1400x1000x24'],pass_fds=(write,),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);os.close(write)
        with os.fdopen(read) as pipe:env['DISPLAY']=':'+pipe.readline().strip()
        for mode in ['cold_database','warm_database']:
            log=root/(mode+'.log');started=time.monotonic()
            with log.open('w') as output:app=subprocess.Popen(['target/debug/rustrss-desktop'],env=env,stdout=output,stderr=output)
            for _ in range(300):
                if 'loaded feeds=' in log.read_text():break
                assert app.poll() is None
                await asyncio.sleep(.02)
            await probe.until("!!document.querySelector('#entries li[data-id]')")
            elapsed=time.monotonic()-started
            assert await probe.js("document.querySelectorAll('#feeds li[data-feed-id]').length") == 500
            report['runs'].append({'mode':mode,'interactive_seconds':elapsed,'database_residency_after':scale.residency(dbpath)})
            app.terminate();app.wait(timeout=10)
        report['passed']=True;print(json.dumps(report),flush=True)
    finally:
        (root/'results.json').write_text(json.dumps(report,indent=2))
        for child in [app,xvfb]:
            if child and child.poll() is None:child.terminate();child.wait(timeout=10)
if __name__=='__main__':asyncio.run(main(sys.argv[1]))
