"""Native window controls against isolated Linux WebKitGTK.

Virtual: dbus-run-session -- /usr/bin/python3 scripts/verify-reading-experience-t6-window.py OUT
Real Wayland: /usr/bin/python3 scripts/verify-reading-experience-t6-window.py OUT --real
Real XWayland: /usr/bin/python3 scripts/verify-reading-experience-t6-window.py OUT --real-x11
"""
import asyncio
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile

sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location('native_probe',Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe=importlib.util.module_from_spec(spec);spec.loader.exec_module(native_probe)
def port():
    with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]

async def main():
    out=Path(sys.argv[1]).resolve();out.mkdir(parents=True,exist_ok=True)
    real='--real' in sys.argv[2:]
    real_x11='--real-x11' in sys.argv[2:]
    root=Path(tempfile.mkdtemp(prefix='rustrss-t6-window-'));(root/'home').mkdir();(root/'runtime').mkdir(mode=0o700)
    db=root/'fixture.sqlite';shutil.copy2('/tmp/rustrss-t4-fixture.sqlite',db)
    assert not real or (os.environ.get('WAYLAND_DISPLAY') and os.environ.get('XDG_RUNTIME_DIR'))
    assert not real_x11 or os.environ.get('DISPLAY')
    env=dict(os.environ,HOME=str(root/'home'),XDG_DATA_HOME=str(root/'data'),RUSTSS_DB=str(db),GDK_GL='disable',GDK_BACKEND='wayland',WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{port()}')
    if real_x11: env['GDK_BACKEND']='x11';env.pop('WAYLAND_DISPLAY',None)
    else: env.pop('DISPLAY',None)
    kwin=None
    if not real and not real_x11:
        env['XDG_RUNTIME_DIR']=str(root/'runtime');env.pop('WAYLAND_DISPLAY',None)
        socket_name='wayland-rustrss-t6-window'
        kwin_log=out/'window-kwin.log'
        kwin=subprocess.Popen(['kwin_wayland','--virtual','--socket',socket_name,'--width','1600','--height','1000','--no-lockscreen','--no-global-shortcuts'],env=env,stdout=kwin_log.open('w'),stderr=subprocess.STDOUT)
    app=None;checks=[]
    def check(name,ok,detail=None):assert ok,f'{name}: {detail}';checks.append({'name':name,'detail':detail})
    try:
        if kwin:
            for _ in range(200):
                if (root/'runtime'/socket_name).exists():break
                assert kwin.poll() is None;await asyncio.sleep(.1)
            env['WAYLAND_DISPLAY']=socket_name
        app=subprocess.Popen(['target/debug/rustrss-desktop'],env=env,stdout=(out/'window-app.log').open('w'),stderr=subprocess.STDOUT)
        probe=native_probe.Probe({'root':str(root),'inspector_port':int(env['WEBKIT_INSPECTOR_HTTP_SERVER'].split(':')[-1])})
        for _ in range(200):
            try:await probe.js('1');break
            except (OSError,IndexError):assert app.poll() is None;await asyncio.sleep(.1)
        await probe.until('!!document.querySelector("#btn-win-max")')
        async def snapshot():
            await probe.js('''window.__T6_WINDOW='pending';Promise.all([window.__TAURI__.window.getCurrentWindow().innerSize(),window.__TAURI__.window.getCurrentWindow().isMinimized(),window.__TAURI__.window.getCurrentWindow().isMaximized(),window.__TAURI__.window.getCurrentWindow().isVisible()]).then(([size,minimized,maximized,visible])=>window.__T6_WINDOW={size,minimized,maximized,visible},e=>window.__T6_WINDOW={error:String(e)});true''')
            await probe.until('window.__T6_WINDOW!=="pending"')
            state=await probe.js('window.__T6_WINDOW');assert 'error' not in state,state
            return state
        before=await snapshot();check('native-window',before['size']['width']>=900,before)
        await probe.js('document.querySelector("#btn-win-max").click();true')
        await asyncio.sleep(.4);maxed=await snapshot()
        check('native-maximize',maxed['maximized'] and maxed['size']['width']>before['size']['width'] and maxed['size']['height']>before['size']['height'],{'before':before,'after':maxed})
        await probe.js('document.querySelector("#btn-win-max").click();true')
        await asyncio.sleep(.4);unmaxed=await snapshot()
        check('native-unmaximize',not unmaxed['maximized'] and unmaxed['size']==before['size'],unmaxed)
        await probe.js('document.querySelector("#btn-win-min").click();true')
        await asyncio.sleep(1.5);minimized=await snapshot()
        checks.append({'name':'native-minimize','passed':minimized['minimized'] is True,'detail':minimized})
        if not minimized['minimized']:
            await probe.js('window.__T6_MIN="pending";window.__TAURI__.core.invoke("window_minimize").then(()=>window.__T6_MIN="ok",e=>window.__T6_MIN=String(e));true')
            await probe.until('window.__T6_MIN!=="pending"')
            direct=await probe.js('window.__T6_MIN')
            await asyncio.sleep(.4)
            checks.append({'name':'native-minimize-direct-ipc-diagnostic','passed':False,'detail':{'result':direct,'after':await snapshot()}})
        await probe.js('window.__T6_RESTORE="pending";window.__TAURI__.window.getCurrentWindow().unminimize().then(()=>window.__T6_RESTORE="ok",e=>window.__T6_RESTORE=String(e));true')
        await probe.until('window.__T6_RESTORE!=="pending"')
        await asyncio.sleep(.4);restored=await snapshot()
        if restored['minimized']:
            await probe.js('window.__T6_SHOW="pending";window.__TAURI__.window.getCurrentWindow().show().then(()=>window.__T6_SHOW="ok",e=>window.__T6_SHOW=String(e));true')
            await probe.until('window.__T6_SHOW!=="pending"')
            await asyncio.sleep(.4);restored=await snapshot()
        checks.append({'name':'native-restore-after-minimize','passed':restored['minimized'] is False,'detail':restored})
        try: await probe.js('document.querySelector("#btn-win-close").click();true')
        except Exception:
            # Closing destroys the WebKit inspector connection before reply.
            pass
        for _ in range(50):
            if app.poll() is not None:break
            await asyncio.sleep(.1)
        check('native-close-exit',app.poll()==0,{'exitCode':app.poll()})
        session='real XWayland display' if real_x11 else 'real Wayland display' if real else 'private dbus-run-session virtual KWin Wayland'
        report={'binarySha256':hashlib.sha256(Path('target/debug/rustrss-desktop').read_bytes()).hexdigest(),'session':session+' with isolated owned app HOME/XDG_DATA_HOME/DB','input':'WebKit button activation; Tauri native window state and process exit readbacks','checks':checks,'passed':all(c.get('passed',True) for c in checks)}
        (out/('window-x11-results.json' if real_x11 else 'window-real-results.json' if real else 'window-results.json')).write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps({'checks':len(checks),'passed':report['passed'],'binarySha256':report['binarySha256']}))
    finally:
        if app and app.poll() is None:app.terminate();app.wait(timeout=10)
        if kwin:kwin.terminate();kwin.wait(timeout=10)

if __name__=='__main__':asyncio.run(main())
