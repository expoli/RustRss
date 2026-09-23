"""Real embedded HTTP + independent stdio writes, observed in rebuilt desktop UI."""
import argparse
import base64
import io
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

parser=argparse.ArgumentParser()
parser.add_argument('--display',choices=['xvfb','wayland'],default='xvfb')
parser.add_argument('--scale',choices=['1','2'],default='1')
options=parser.parse_args()
subprocess.run(['df','-h','.'],check=True)
root=Path(tempfile.mkdtemp(prefix='rustrss-theme-preview-'))
dbpath=root/'fixture.sqlite'
subprocess.run(['target/debug/examples/theme_fixture',str(dbpath)],check=True)
with socket.socket() as sock:
    sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
with sqlite3.connect(dbpath) as db:
    for k,v in [('mcp.enabled','true'),('mcp.port',str(port)),('mcp.token','fixture-read'),('mcp.write_token','fixture-write'),('mcp.write_enabled','true')]:
        db.execute('INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES(?,?,0)',(k,v))
xvfb=None
env=dict(os.environ,RUSTSS_DB=str(dbpath),RUSTSS_LOG_STDOUT='1',RUSTSS_AI_KEY='isolated-fixture-placeholder',XDG_DATA_HOME=str(root/'data'))
if options.display=='xvfb':
    r,w=os.pipe();xvfb=subprocess.Popen(['Xvfb','-displayfd',str(w),'-screen','0','2560x1800x24'],pass_fds=(w,),stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL);os.close(w)
    with os.fdopen(r) as pipe:display=':'+pipe.readline().strip()
    runtime=root/'runtime';runtime.mkdir(mode=0o700)
    env.update(DISPLAY=display,GDK_GL='disable',GDK_SCALE=options.scale,XDG_RUNTIME_DIR=str(runtime))
    for k in ['GDK_BACKEND','WAYLAND_DISPLAY','EGL_PLATFORM']:env.pop(k,None)
else:
    if not env.get('WAYLAND_DISPLAY'):raise RuntimeError('No native Wayland session available')
    env.pop('DISPLAY',None)
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
    with urllib.request.urlopen(request,timeout=10) as response:return json.load(response)['result']
def stdio_call(method,params,id):
    stdio.stdin.write(json.dumps({'jsonrpc':'2.0','id':id,'method':method,'params':params})+'\n');stdio.stdin.flush()
    ready,_,_=select.select([stdio.stdout],[],[],15)
    if not ready:raise RuntimeError('stdio reply timeout')
    return json.loads(stdio.stdout.readline())
def shot(name):
    run('xdotool','windowsize',window,'1239','820');run('xdotool','windowsize',window,'1240','820');time.sleep(.2)
    run('import','-window',window,str(root/name))
def metadata(result):
    value=result['structuredContent']
    assert value['ok'],value
    return value

def capture(result,label):
    value=metadata(result)
    images=[c for c in result['content'] if c['type']=='image']
    assert len(images)==1 and images[0]['mimeType']=='image/png'
    data=base64.b64decode(images[0]['data']);assert len(data)<=2*1024*1024
    (root/(label+'.png')).write_bytes(data)
    image=Image.open(io.BytesIO(data)).convert('RGB')
    assert list(image.size)==value['capture']['pixel_size']
    assert value['capture']['freshness_marker_verified']
    captures.append({'label':label,'bytes':len(data),'revision':value['preview_revision'],'hash':value['config_hash'],'capture_ms':value['capture']['capture_ms'],'logical_size':value['capture']['logical_size'],'pixel_size':value['capture']['pixel_size'],'scale_factor':value['capture']['scale_factor'],'display_backend':value['capture']['display_backend']})
    return value,image

def done(p,action):return {'preview_id':p['preview_id'],'expected_preview_revision':p['preview_revision'],'action':action}
captures=[]
try:
    with logfile.open('w') as output:app=subprocess.Popen(['target/debug/rustrss-desktop'],env=env,stdout=output,stderr=output)
    wait_for(lambda:'loaded feeds=' in logfile.read_text(),'desktop boot')
    if options.display=='xvfb':
        window=run('xdotool','search','--onlyvisible','--name','^RustRss$').splitlines()[0]
        run('xdotool','windowfocus',window,'mousemove',str(330*int(options.scale)),str(150*int(options.scale)),'click','1')
        wait_for(lambda:'renderReader id=' in logfile.read_text(),'article opened')
    initial_renders=logfile.read_text().count('renderReader id=')
    assert metadata(http('get_theme',{},'fixture-read'))['capabilities']['preview']['available']
    denied=http('preview_theme',{'base_revision':0,'patch':{}},'fixture-read');assert denied['isError']
    p=None
    for preset in ['clear','paper','slate']:
      for mode in ['light','dark']:
        for scene in ['overview','article','settings']:
          args={'base_revision':0,'patch':{'light_preset':preset,'dark_preset':preset},'scene':scene,'mode':mode}
          if p:args.update(preview_id=p['preview_id'],expected_preview_revision=p['preview_revision'])
          p,img=capture(http('preview_theme',args),f'{preset}-{mode}-{scene}')
          if scene=='article':
            resolved=metadata(http('validate_theme',{'expected_revision':0,'patch':args['patch']},'fixture-read'))['theme'][mode]
            scale=p['capture']['scale_factor'];w,h=p['capture']['logical_size']
            assert img.getpixel(((w-20)*scale,(h-100)*scale))==tuple(bytes.fromhex(resolved['colors']['background'][1:])),(preset,mode,img.size)
    p,_=capture(http('capture_theme_preview',{'preview_id':p['preview_id'],'expected_preview_revision':p['preview_revision'],'scene':'overview','mode':'light'}),'recapture')
    with sqlite3.connect(dbpath) as db:assert db.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone() is None
    metadata(http('finish_theme_preview',done(p,'cancel')))
    for cycle in range(3):
        p,_=capture(http('preview_theme',{'base_revision':0,'patch':{}}),f'reopen-{cycle}')
        metadata(http('finish_theme_preview',done(p,'cancel')))
    # Independent binary bridges image requests to this profile's desktop service.
    with (root/'stdio.log').open('w') as errors:stdio=subprocess.Popen(['target/debug/rustrss-mcp'],env=env,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=errors,text=True,bufsize=1)
    stdio_call('initialize',{'protocolVersion':'2026-07-28','capabilities':{},'clientInfo':{'name':'preview-verifier','version':'1'}},1)
    stdio.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'})+'\n');stdio.stdin.flush()
    reply=stdio_call('tools/call',{'name':'preview_theme','arguments':{'base_revision':0,'patch':{'mode':'light','light_preset':'paper'},'scene':'article','mode':'light'}},2)['result']
    p,img=capture(reply,'stdio-paper')
    saved=metadata(http('finish_theme_preview',done(p,'save')));assert saved['saved_revision']==1
    assert metadata(http('finish_theme_preview',done(p,'save')))==saved
    wait_for(lambda:'theme applied revision=1 ' in logfile.read_text(),'saved theme applied to main reader')
    with sqlite3.connect(dbpath) as db:
      current=json.loads(db.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone()[0])['current']
    assert current['revision']==1 and current['light_preset']=='paper'
    assert logfile.read_text().count('renderReader id=')==initial_renders
    if options.display=='xvfb':
        # Local close cancels independently of the MCP credential.
        p,_=capture(http('preview_theme',{'base_revision':1,'patch':{'mode':'dark'},'mode':'dark'}),'local-close')
        preview=run('xdotool','search','--onlyvisible','--name','RustRss — Theme preview').splitlines()[0]
        run('xdotool','windowfocus',preview,'mousemove','--window',preview,str(1190*int(options.scale)),str(877*int(options.scale)),'click','1')
        time.sleep(.3)
        expired=http('finish_theme_preview',done(p,'save'));assert expired['isError'],expired
    # Credential revocation reaps the window without requiring another tool request.
    p,_=capture(http('preview_theme',{'base_revision':1,'patch':{}}),'revoke')
    with sqlite3.connect(dbpath) as db:db.execute("UPDATE settings SET value='false' WHERE key='mcp.write_enabled'")
    if options.display=='xvfb':
        def closed():
            return subprocess.run(['xdotool','search','--onlyvisible','--name','RustRss — Theme preview'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL).returncode!=0
        wait_for(closed,'revoked preview window reclaimed',seconds=8)
    else:
        time.sleep(6)
    # Re-enable writes and confirm the revoked session cannot be saved.
    with sqlite3.connect(dbpath) as db:db.execute("UPDATE settings SET value='true' WHERE key='mcp.write_enabled'")
    assert http('finish_theme_preview',done(p,'save'))['isError']
    report={'output':str(root),'captures':captures,'temporary_no_write':True,'stdio_bridge_image':True,'saved_revision':1,'idempotent_save':True,'display':options.display,'requested_scale':options.scale if options.display=='xvfb' else None,'local_cancel':options.display=='xvfb','revocation_reaped':True,'article_renders_before_after':[initial_renders,logfile.read_text().count('renderReader id=')]}
    (root/'results.json').write_text(json.dumps(report,indent=2));print(json.dumps({k:v for k,v in report.items() if k!='captures'}));print('captures:',len(captures))
finally:
    for child in [stdio,app]:
        if child and child.poll() is None:
            child.terminate()
            try:child.wait(timeout=10)
            except subprocess.TimeoutExpired:child.kill();child.wait(timeout=10)
    if xvfb:xvfb.terminate();xvfb.wait(timeout=10)
    print('Evidence:',root)
