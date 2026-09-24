"""Real 600s idle / 1800s absolute preview expiry; no test clocks or shortened TTLs.
Two isolated profiles run concurrently in one Xvfb. The probe retains no PNG copies;
the desktop manages returned screenshot files with its bounded TTL.
"""
import json
import os
from pathlib import Path
import shutil
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.request

root = Path(tempfile.mkdtemp(prefix='rustrss-t7-lifetime-'))
print('Evidence:', root, flush=True)
subprocess.run(['df', '-h', '.'], check=True)
r, w = os.pipe()
xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(w), '-screen', '0', '1400x1000x24'], pass_fds=(w,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
os.close(w)
with os.fdopen(r) as pipe:
    display = ':' + pipe.readline().strip()
profiles = []
report = {'wall_clock': True, 'profiles': [], 'samples': []}

def wait_for(fn, reason, seconds=20):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if fn(): return
        time.sleep(.1)
    raise RuntimeError(reason)

def call(p, name, args):
    payload = {'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':name,'arguments':args}}
    req = urllib.request.Request(f"http://127.0.0.1:{p['port']}/mcp", data=json.dumps(payload).encode(), headers={'Authorization':'Bearer fixture-write','Content-Type':'application/json','Accept':'application/json, text/event-stream'})
    with urllib.request.urlopen(req, timeout=15) as response:
        return json.load(response)['result']

def value(result):
    v = result['structuredContent']
    assert v['ok'], v
    return v

def frame(p, result):
    v = value(result)
    assert all(c['type'] == 'text' for c in result['content'])
    path = Path(v['image_path'])
    assert path.is_absolute() and v['image_expires_at_ms'] > time.time()*1000
    data = path.read_bytes()
    assert len(data) == v['image_bytes']
    assert len(data) <= 2 * 1024 * 1024 and data.startswith(b'\x89PNG')
    assert v['capture']['freshness_marker_verified']
    assert v['capture']['display_backend'] == 'GdkX11Display'
    p['captures'].append({'elapsed':time.monotonic()-p['start'], 'bytes':len(data), 'capture_ms':v['capture']['capture_ms']})
    return v

def windows(p):
    output = subprocess.run(['xdotool','search','--onlyvisible','--pid',str(p['app'].pid)], env=p['env'], capture_output=True, text=True, timeout=5)
    result = []
    for wid in output.stdout.split():
        name = subprocess.run(['xdotool','getwindowname',wid],env=p['env'],capture_output=True,text=True,timeout=5).stdout.strip()
        result.append({'id':wid,'name':name})
    return result

def preview_visible(p):
    return any(w['name'] == 'RustRss — Theme preview' for w in windows(p))

def memory(pid):
    # Include the WebKit children; RSS double-counts shared pages and is not PSS.
    parents = {}
    rss = {}
    for proc in Path('/proc').iterdir():
        if not proc.name.isdigit(): continue
        try:
            fields = dict(line.split(':',1) for line in (proc/'status').read_text().splitlines() if ':' in line)
            parents[int(proc.name)] = int(fields['PPid'])
            rss[int(proc.name)] = int(fields.get('VmRSS','0 kB').split()[0])
        except (OSError, ValueError, KeyError): continue
    descendants = {pid}
    while True:
        found = {child for child, parent in parents.items() if parent in descendants}
        if found <= descendants: break
        descendants |= found
    return {'rss_kib':sum(rss.get(p,0) for p in descendants),'processes':len(descendants)}

def disk_bytes(path):
    return sum(p.stat().st_size for p in path.rglob('*') if p.is_file())

try:
    for kind in ['idle','absolute']:
        directory = root/kind
        directory.mkdir()
        db = directory/'fixture.sqlite'
        subprocess.run(['target/debug/examples/theme_fixture',str(db)],check=True)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1',0)); port=sock.getsockname()[1]
        with sqlite3.connect(db) as conn:
            for k,v in [('mcp.enabled','true'),('mcp.port',str(port)),('mcp.token','fixture-read'),('mcp.write_enabled','true'),('mcp.write_token','fixture-write')]:
                conn.execute('INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES(?,?,0)',(k,v))
        runtime=directory/'runtime'
        runtime.mkdir(mode=0o700)
        env=dict(os.environ,XDG_RUNTIME_DIR=str(runtime),DISPLAY=display,GDK_GL='disable',GDK_SCALE='1',HOME=str(directory/'home'),XDG_DATA_HOME=str(directory/'data'),RUSTSS_DB=str(db),RUSTSS_LOG_STDOUT='1',RUSTSS_AI_KEY='isolated-fixture-placeholder')
        for k in ['GDK_BACKEND','WAYLAND_DISPLAY','EGL_PLATFORM']:env.pop(k,None)
        log=directory/'desktop.log'
        with log.open('w') as out: app=subprocess.Popen(['target/debug/rustrss-desktop'],env=env,stdout=out,stderr=out)
        p={'kind':kind,'directory':directory,'db':db,'env':env,'port':port,'app':app,'captures':[]}
        profiles.append(p)
        wait_for(lambda:'loaded feeds=' in log.read_text(),'desktop boot')
        p['start']=time.monotonic()
        p['preview']=frame(p,call(p,'preview_theme',{'base_revision':0,'patch':{'light_preset':'paper'},'scene':'article','mode':'light'}))
        wait_for(lambda:preview_visible(p),'preview window association')
        p['next_capture']=120
        p['closed_at']=None
        print(kind, 'started', 'pid',app.pid,flush=True)
    start=time.monotonic()
    next_sample=0
    while time.monotonic()-start < 1830:
        now=time.monotonic()
        for p in profiles:
            assert p['app'].poll() is None, 'desktop exited'
            elapsed=now-p['start']
            if p['kind']=='absolute' and elapsed >= p['next_capture'] and elapsed < 1790:
                p['preview']=frame(p,call(p,'capture_theme_preview',{'preview_id':p['preview']['preview_id'],'expected_preview_revision':p['preview']['preview_revision']}))
                p['next_capture']+=120
            limit=600 if p['kind']=='idle' else 1800
            if p['closed_at'] is None:
                visible=preview_visible(p)
                if not visible:
                    p['closed_at']=elapsed
                    assert limit-2 <= elapsed <= limit+12, (p['kind'],elapsed)
                    expired=call(p,'finish_theme_preview',{'preview_id':p['preview']['preview_id'],'expected_preview_revision':p['preview']['preview_revision'],'action':'save'})
                    assert expired.get('isError'), expired
                    p['expiry_error']=expired.get('structuredContent')
                    print(p['kind'],'reaped at',round(elapsed,2),'seconds',flush=True)
        if now-start >= next_sample:
            free=shutil.disk_usage('.').free
            if free < 10*1024**3: raise RuntimeError('disk below 10GiB; aborting soak')
            sample={'elapsed':now-start,'free_bytes':free,'profiles':[]}
            for p in profiles:
                with sqlite3.connect(p['db']) as conn:
                    assert conn.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone() is None
                sample['profiles'].append({'kind':p['kind'],**memory(p['app'].pid),'disk_bytes':disk_bytes(p['directory']),'windows':windows(p)})
            report['samples'].append(sample)
            with (root/'samples.jsonl').open('a') as out:out.write(json.dumps(sample)+'\n')
            if int(next_sample)%120==0:
                subprocess.run(['df','-h','.'],check=True)
                print('elapsed',round(now-start),'seconds',flush=True)
            next_sample+=30
        if all(p['closed_at'] is not None for p in profiles):break
        time.sleep(2)
    assert all(p['closed_at'] is not None for p in profiles), 'expiry not observed'
    for p in profiles:
        report['profiles'].append({'kind':p['kind'],'closed_at_seconds':p['closed_at'],'captures':p['captures'],'expiry_error':p['expiry_error'],'temporary_no_write':True})
    report['ok']=True
except Exception as error:
    report['error']=repr(error)
    raise
finally:
    (root/'results.json').write_text(json.dumps(report,indent=2))
    for p in profiles:
        app=p['app']
        if app.poll() is None:
            app.terminate()
            try:app.wait(timeout=10)
            except subprocess.TimeoutExpired:app.kill();app.wait(timeout=10)
    xvfb.terminate();xvfb.wait(timeout=10)
    print('Evidence:',root,flush=True)
