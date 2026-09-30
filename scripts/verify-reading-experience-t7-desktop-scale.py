"""Final Linux WebKitGTK at real GTK 200% scale in isolated Xvfb."""
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
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe = importlib.util.module_from_spec(spec); spec.loader.exec_module(native_probe)

def port():
    with socket.socket() as s:
        s.bind(('127.0.0.1',0)); return s.getsockname()[1]

async def main():
    out=Path(sys.argv[1]).resolve(); out.mkdir(parents=True,exist_ok=True)
    root=Path(tempfile.mkdtemp(prefix='rustrss-t6-layout-'))
    (root/'home').mkdir(); (root/'runtime').mkdir(mode=0o700)
    db=root/'fixture.sqlite'; shutil.copy2('/tmp/rustrss-t4-fixture.sqlite',db)
    env=dict(os.environ,HOME=str(root/'home'),XDG_DATA_HOME=str(root/'data'),XDG_RUNTIME_DIR=str(root/'runtime'),RUSTSS_DB=str(db),GDK_GL='disable',GDK_SCALE='2',GDK_DPI_SCALE='1',WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{port()}')
    env.pop('DISPLAY',None); env.pop('WAYLAND_DISPLAY',None)
    display=next(':'+str(i) for i in range(70,90) if not Path(f'/tmp/.X11-unix/X{i}').exists())
    xvfb=subprocess.Popen(['/tmp/rustrss-t4-xvfb/root/usr/bin/Xvfb',display,'-screen','0','3200x2000x24','-nolisten','tcp','-ac'],env=env,stdout=(out/'scale-xvfb.log').open('w'),stderr=subprocess.STDOUT)
    app=None
    try:
        for _ in range(200):
            if Path('/tmp/.X11-unix/X'+display[1:]).exists(): break
            assert xvfb.poll() is None
            await asyncio.sleep(.1)
        env.update(DISPLAY=display,GDK_BACKEND='x11')
        app=subprocess.Popen(['target/debug/rustrss-desktop'],env=env,stdout=(out/'scale-app.log').open('w'),stderr=subprocess.STDOUT)
        probe=native_probe.Probe({'root':str(root),'inspector_port':int(env['WEBKIT_INSPECTOR_HTTP_SERVER'].split(':')[-1])})
        for _ in range(200):
            try: await probe.js('1'); break
            except (OSError,IndexError): assert app.poll() is None; await asyncio.sleep(.1)
        await probe.until('!!document.querySelector("#entries li[data-id=\\"1\\"]")')
        listing=subprocess.check_output(['xwininfo','-root','-tree'],env=env,text=True)
        found=re.search(r'(0x[0-9a-f]+) "RustRss"',listing); assert found,listing
        window=int(found.group(1),16)
        x11=ctypes.CDLL('libX11.so.6'); x11.XOpenDisplay.argtypes=[ctypes.c_char_p];x11.XOpenDisplay.restype=ctypes.c_void_p
        x11.XResizeWindow.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.c_uint,ctypes.c_uint]
        x11.XSync.argtypes=[ctypes.c_void_p,ctypes.c_int]
        handle=x11.XOpenDisplay(display.encode()); assert handle
        results=[]
        for width in [2560,2880]:
            x11.XResizeWindow(handle,window,width,1700);x11.XSync(handle,0);await asyncio.sleep(.35)
            data=await probe.js('''(() => {const b=s=>{const n=document.querySelector(s),r=n.getBoundingClientRect();return {x:r.x,y:r.y,w:r.width,h:r.height,display:getComputedStyle(n).display}};return {width:innerWidth,height:innerHeight,dpr:devicePixelRatio,scrollWidth:document.documentElement.scrollWidth,coarse:matchMedia('(pointer:coarse)').matches,mobile:matchMedia('(max-width:960px) and (pointer:coarse)').matches,nav:b('#m-nav'),columns:[b('.sidebar'),b('.list'),b('.right-col')],toolbar:['#btn-refresh','#btn-add','#btn-settings','#btn-win-min','#btn-win-max','#btn-win-close'].map(b),grid:getComputedStyle(document.querySelector('main')).gridTemplateColumns,firstTitle:b('#entries li[data-id] .title')}})()''')
            assert data['width']==width//2 and data['dpr']==2,data
            assert not data['coarse'] and not data['mobile'] and data['nav']['w']==0,data
            assert data['scrollWidth']<=data['width']+1,data
            assert all(c['w']>0 and c['x']>=-1 and c['x']+c['w']<=data['width']+1 for c in data['columns']),data
            assert all(b['w']>0 and b['x']>=-1 and b['x']+b['w']<=data['width']+1 for b in data['toolbar']),data
            await probe.js('document.querySelector("#btn-settings").click();true')
            modal=await probe.js('''(() => {const r=document.querySelector('.settings-dialog').getBoundingClientRect();return {x:r.x,y:r.y,w:r.width,h:r.height,role:document.querySelector('#settings-overlay').getAttribute('role'),mcp:!!document.querySelector('#tab-mcp').getClientRects().length}})()''')
            assert modal['x']>=0 and modal['x']+modal['w']<=data['width']+1 and modal['mcp'] and modal['role']=='dialog',modal
            await probe.js('document.querySelector("#settings-close").click();true')
            results.append({'physicalWidth':width,**data,'settings':modal})
            subprocess.run(['import','-display',display,'-window',hex(window),str(out/f'desktop-200pct-{data["width"]}.png')],env=env,check=True)
        report={'binarySha256':hashlib.sha256(Path('target/debug/rustrss-desktop').read_bytes()).hexdigest(),'fixtureSha256':hashlib.sha256(Path('/tmp/rustrss-t4-fixture.sqlite').read_bytes()).hexdigest(),'gtkScale':env['GDK_SCALE'],'checks':results,'passed':True}
        (out/'desktop-scale-results.json').write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps({'widths':len(results),'binarySha256':report['binarySha256']}))
    finally:
        if app and app.poll() is None: app.terminate();app.wait(timeout=10)
        xvfb.terminate();xvfb.wait(timeout=10)

if __name__=='__main__': asyncio.run(main())
