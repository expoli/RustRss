"""Real embedded HTTP + independent stdio writes, observed in rebuilt desktop UI."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import select
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.request
from PIL import Image

root=Path(tempfile.mkdtemp(prefix='rustrss-theme-mcp-'))
dbpath=root/'fixture.sqlite'
subprocess.run(['target/debug/examples/theme_fixture',str(dbpath)],check=True)
with socket.socket() as sock:
    sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
with sqlite3.connect(dbpath) as db:
    for k,v in [('mcp.enabled','true'),('mcp.port',str(port)),('mcp.token','fixture-read'),('mcp.write_token','fixture-write'),('mcp.write_enabled','true')]:
        db.execute('INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES(?,?,0)',(k,v))
r,w=os.pipe();xvfb=subprocess.Popen(['Xvfb','-displayfd',str(w),'-screen','0','1240x900x24'],pass_fds=(w,),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);os.close(w)
with os.fdopen(r) as pipe: display=':'+pipe.readline().strip()
runtime=root/'runtime';runtime.mkdir(mode=0o700)
env=dict(os.environ,DISPLAY=display,GDK_GL='disable',GDK_SCALE='1',XDG_RUNTIME_DIR=str(runtime),XDG_DATA_HOME=str(root/'data'),RUSTSS_DB=str(dbpath),RUSTSS_LOG_STDOUT='1',RUSTSS_AI_KEY='isolated-fixture-placeholder')
for k in ['GDK_BACKEND','WAYLAND_DISPLAY','EGL_PLATFORM']:env.pop(k,None)
app=None;stdio=None
logfile=root/'desktop.log'
def wait_for(fn,reason,seconds=15):
    end=time.monotonic()+seconds
    while time.monotonic()<end:
        if fn():return
        time.sleep(.1)
    raise RuntimeError(reason)
def run(*args):return subprocess.check_output(args,env=env,text=True,timeout=10).strip()
def body(reply):return json.loads(reply['result']['content'][0]['text'])
def http(name,args,token='fixture-write'):
    payload={'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':name,'arguments':args}}
    request=urllib.request.Request(f'http://127.0.0.1:{port}/mcp',data=json.dumps(payload).encode(),headers={'Authorization':'Bearer '+token,'Content-Type':'application/json','Accept':'application/json, text/event-stream'})
    with urllib.request.urlopen(request,timeout=10) as response:return body(json.load(response))
def stdio_call(method,params,id):
    stdio.stdin.write(json.dumps({'jsonrpc':'2.0','id':id,'method':method,'params':params})+'\n');stdio.stdin.flush()
    ready,_,_=select.select([stdio.stdout],[],[],15)
    if not ready:raise RuntimeError('stdio reply timeout')
    return json.loads(stdio.stdout.readline())
def shot(name):
    run('xdotool','windowsize',window,'1239','820');run('xdotool','windowsize',window,'1240','820');time.sleep(.2)
    run('import','-window',window,str(root/name))
try:
    with logfile.open('w') as output:app=subprocess.Popen(['target/debug/rustrss-desktop'],env=env,stdout=output,stderr=output)
    wait_for(lambda:'loaded feeds=' in logfile.read_text(),'desktop boot')
    window=run('xdotool','search','--onlyvisible','--name','^RustRss$').splitlines()[0]
    # Keyboard instead of a coordinate click: later batches changed the list markup, so the
    # old (330,120) point no longer lands on a row, and the app's own bindings are stable.
    run('xdotool','windowfocus',window)
    run('xdotool','key','j'); time.sleep(.3); run('xdotool','key','Return')
    wait_for(lambda:'renderReader id=' in logfile.read_text(),'article opened')
    renders=logfile.read_text().count('renderReader id=')
    g=http('get_theme',{},'fixture-read');assert g['capabilities']['change_notification'] is True
    denied=http('update_theme',{'expected_revision':0,'patch':{'mode':'dark'}},'fixture-read');assert denied['error_code']=='write_scope_required'
    # Hidden/unfocused UI cannot use foreground polling: this must be an event.
    run('xdotool','windowunmap',window);time.sleep(.3)
    saved=http('update_theme',{'expected_revision':0,'patch':{'mode':'dark','dark_preset':'slate'}})
    assert saved['detail']['live_apply']=='pending'
    wait_for(lambda:'theme applied revision=1 ' in logfile.read_text(),'embedded event apply')
    run('xdotool','windowmap',window,'windowfocus',window)
    shot('embedded-slate.png')
    assert Image.open(root/'embedded-slate.png').convert('RGB').getpixel((1200,730))==(36,45,49)
    restored=http('restore_theme',{'expected_revision':1,'historical_revision':0});assert restored['detail']['saved_revision']==2
    wait_for(lambda:'theme applied revision=2 ' in logfile.read_text(),'restore applied')
    with (root/'stdio.log').open('w') as errors:stdio=subprocess.Popen(['target/debug/rustrss-mcp'],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=errors,text=True,bufsize=1)
    stdio_call('initialize',{'protocolVersion':'2026-07-28','capabilities':{},'clientInfo':{'name':'theme-fixture','version':'1'}},1)
    stdio.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'})+'\n');stdio.stdin.flush()
    reply=body(stdio_call('tools/call',{'name':'update_theme','arguments':{'expected_revision':2,'patch':{'mode':'light','light_preset':'paper'}}},2))
    assert reply['detail']['saved_revision']==3 and reply['detail']['live_apply']=='unavailable'
    wait_for(lambda:'theme applied revision=3 ' in logfile.read_text(),'independent stdio polling apply')
    shot('stdio-paper.png')
    assert Image.open(root/'stdio-paper.png').convert('RGB').getpixel((1200,730))==(255,253,247)
    assert logfile.read_text().count('renderReader id=')==renders,'theme sync rebuilt article'
    with sqlite3.connect(dbpath) as db:current=json.loads(db.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone()[0])['current']
    assert current['revision']==3 and current['light_preset']=='paper'
    binary=Path('target/debug/rustrss-desktop')
    report={'output':str(root),
            'binary':str(binary),
            'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
            'binary_mtime':datetime.datetime.fromtimestamp(binary.stat().st_mtime).isoformat(timespec='seconds'),
            'captured_at':datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),'embedded_event':True,'stdio_polling':True,'restore':True,'read_token_rejected':True,'article_renders_before_after':[renders,logfile.read_text().count('renderReader id=')],'revision':3,'pixel_checks':2,'preview_available':g['capabilities']['preview']['available']}
    (root/'results.json').write_text(json.dumps(report,indent=2));print(json.dumps(report))
finally:
    for child in [stdio,app]:
        if child and child.poll() is None:
            child.terminate()
            try:child.wait(timeout=10)
            except subprocess.TimeoutExpired:child.kill();child.wait(timeout=10)
    xvfb.terminate();xvfb.wait(timeout=10)
    print('Evidence:',root)
