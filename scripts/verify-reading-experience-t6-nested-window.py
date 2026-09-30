"""Trusted host-Xvfb pointer into an owned nested KWin/RustRss session."""
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
import time
import uuid

sys.dont_write_bytecode=True
spec=importlib.util.spec_from_file_location('native_probe',Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe=importlib.util.module_from_spec(spec);spec.loader.exec_module(native_probe)

def free_port():
    with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]
def run(*args,env=None):return subprocess.check_output(args,env=env,text=True,timeout=15).strip()

async def main():
    out=Path(sys.argv[1]).resolve();out.mkdir(parents=True,exist_ok=True)
    assert os.environ.get('DBUS_SESSION_BUS_ADDRESS'),'Private dbus-run-session required'
    root=Path(os.environ['RUSTSS_T6_NESTED_ROOT'])
    assert os.environ['HOME']==str(root/'home') and os.environ['XDG_DATA_HOME']==str(root/'data')
    db=root/'fixture.sqlite';shutil.copy2('/tmp/rustrss-t4-fixture.sqlite',db)
    display=next(':'+str(i) for i in range(70,90) if not Path(f'/tmp/.X11-unix/X{i}').exists())
    xvfb_path=os.environ.get('T4_XVFB') or shutil.which('Xvfb') or '/tmp/rustrss-t4-xvfb/root/usr/bin/Xvfb'
    host_env=dict(os.environ,HOME=str(root/'home'),XDG_DATA_HOME=str(root/'data'),XDG_CONFIG_HOME=str(root/'config'),XDG_CACHE_HOME=str(root/'cache'),XDG_RUNTIME_DIR=str(root/'runtime'),DISPLAY=display)
    host_env.pop('WAYLAND_DISPLAY',None)
    xvfb=subprocess.Popen([xvfb_path,display,'-screen','0','1600x1000x24','-nolisten','tcp','-ac'],env=host_env,stdout=(out/'nested-xvfb.log').open('w'),stderr=subprocess.STDOUT)
    kwin=None;app=None;scripts=[]
    try:
        for _ in range(100):
            assert xvfb.poll() is None
            if Path('/tmp/.X11-unix/X'+display[1:]).exists():break
            await asyncio.sleep(.1)
        assert Path('/tmp/.X11-unix/X'+display[1:]).exists()
        socket_name='wayland-rustrss-t6-'+uuid.uuid4().hex[:8]
        kwin=subprocess.Popen(['kwin_wayland','--x11-display',display,'--width','1600','--height','1000','--socket',socket_name,'--no-lockscreen','--no-global-shortcuts'],env=host_env,stdout=(out/'nested-kwin.log').open('w'),stderr=subprocess.STDOUT)
        for _ in range(150):
            assert kwin.poll() is None, 'Nested KWin exited'
            if (root/'runtime'/socket_name).exists():break
            await asyncio.sleep(.1)
        assert (root/'runtime'/socket_name).exists(),'Nested Wayland socket absent'
        await asyncio.sleep(.5)
        host_listing=run('xwininfo','-root','-tree',env=host_env)
        (out/'nested-host-windows.txt').write_text(host_listing+'\n')
        host_matches=re.findall(r'(0x[0-9a-f]+) "KDE Wayland Compositor',host_listing)
        assert len(host_matches)==1,host_listing
        host_window=host_matches[0]
        app_env=dict(host_env,WAYLAND_DISPLAY=socket_name,GDK_BACKEND='wayland',GDK_GL='disable',RUSTSS_DB=str(db),WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{free_port()}')
        app_env.pop('DISPLAY',None)
        app=subprocess.Popen(['target/debug/rustrss-desktop'],env=app_env,stdout=(out/'nested-app.log').open('w'),stderr=subprocess.STDOUT)
        probe=native_probe.Probe({'root':str(root),'inspector_port':int(app_env['WEBKIT_INSPECTOR_HTTP_SERVER'].split(':')[-1])})
        for _ in range(200):
            try:await probe.js('1');break
            except (OSError,IndexError):assert app.poll() is None;await asyncio.sleep(.1)
        await probe.until('!!document.querySelector("#btn-win-min")')
        await asyncio.sleep(.5)
        def nested_kwin(action=''):
            tag='T6NEST'+uuid.uuid4().hex
            path=root/(tag+'.js')
            path.write_text('for (const w of workspace.windowList()) if (w.pid === '+str(app.pid)+') { try { '+action+' print("'+tag+' "+JSON.stringify({pid:w.pid,internalId:w.internalId,minimized:w.minimized,active:workspace.activeWindow?.internalId===w.internalId,skipTaskbar:w.skipTaskbar,frame:w.frameGeometry})); } catch(e) { print("'+tag+' "+JSON.stringify({error:String(e),pid:w.pid})); } }\n')
            sid=run('qdbus6','org.kde.KWin','/Scripting','org.kde.kwin.Scripting.loadScript',str(path),tag,env=host_env)
            scripts.append(tag)
            run('qdbus6','org.kde.KWin','/Scripting/Script'+sid,'org.kde.kwin.Script.run',env=host_env)
            for _ in range(40):
                lines=[line.split(tag+' ',1)[1] for line in (out/'nested-kwin.log').read_text().splitlines() if tag+' ' in line]
                if lines:
                    result=json.loads(lines[-1]);assert not result.get('error'),result
                    return result
                time.sleep(.1)
            raise AssertionError('Nested KWin did not report owned app PID '+str(app.pid))
        before=nested_kwin('workspace.activeWindow=w; ')
        assert before['pid']==app.pid and not before['minimized'] and before['active'],before
        button=await probe.js('''(() => {const el=document.querySelector('#btn-win-min');const r=el.getBoundingClientRect();window.__T6_NEST_CLICK=[];el.addEventListener('click',e=>window.__T6_NEST_CLICK.push({trusted:e.isTrusted,detail:e.detail,currentTarget:e.currentTarget.id}),true);return {x:r.x+r.width/2,y:r.y+r.height/2,dpr:devicePixelRatio}})()''')
        host_geometry=run('xwininfo','-id',host_window,env=host_env)
        host_x=int(re.search(r'Absolute upper-left X:\s*(-?\d+)',host_geometry).group(1));host_y=int(re.search(r'Absolute upper-left Y:\s*(-?\d+)',host_geometry).group(1))
        target_x=round(host_x+before['frame']['x']+button['x']*button['dpr'])
        target_y=round(host_y+before['frame']['y']+button['y']*button['dpr'])
        assert 0<=target_x<1600 and 0<=target_y<1000,(target_x,target_y)
        subprocess.run(['import','-display',display,'-window',host_window,str(out/'nested-before.png')],env=host_env,check=True)
        x11=ctypes.CDLL('libX11.so.6');xtst=ctypes.CDLL('libXtst.so.6')
        x11.XOpenDisplay.argtypes=[ctypes.c_char_p];x11.XOpenDisplay.restype=ctypes.c_void_p
        x11.XSync.argtypes=[ctypes.c_void_p,ctypes.c_int]
        x11.XQueryPointer.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.POINTER(ctypes.c_ulong),ctypes.POINTER(ctypes.c_ulong),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_uint)]
        xtst.XTestFakeMotionEvent.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.c_int,ctypes.c_int,ctypes.c_ulong]
        xtst.XTestFakeButtonEvent.argtypes=[ctypes.c_void_p,ctypes.c_uint,ctypes.c_int,ctypes.c_ulong]
        xdisplay=x11.XOpenDisplay(display.encode());assert xdisplay
        assert xtst.XTestFakeMotionEvent(xdisplay,-1,target_x,target_y,0)
        x11.XSync(xdisplay,0);await asyncio.sleep(.2)
        root_return=ctypes.c_ulong();child_return=ctypes.c_ulong();rx=ctypes.c_int();ry=ctypes.c_int();wx=ctypes.c_int();wy=ctypes.c_int();mask=ctypes.c_uint()
        x11.XQueryPointer(xdisplay,int(host_window,16),ctypes.byref(root_return),ctypes.byref(child_return),ctypes.byref(rx),ctypes.byref(ry),ctypes.byref(wx),ctypes.byref(wy),ctypes.byref(mask))
        assert abs(rx.value-target_x)<=2 and abs(ry.value-target_y)<=2,{'pointer':[rx.value,ry.value],'target':[target_x,target_y]}
        assert xtst.XTestFakeButtonEvent(xdisplay,1,1,0)
        assert xtst.XTestFakeButtonEvent(xdisplay,1,0,0)
        x11.XSync(xdisplay,0);await asyncio.sleep(.5)
        clicks=await probe.js('window.__T6_NEST_CLICK')
        after=nested_kwin()
        assert clicks and clicks[-1]=={'trusted':True,'detail':1,'currentTarget':'btn-win-min'},clicks
        assert after['internalId']==before['internalId'] and after['minimized'] is True and after['active'] is False,after
        subprocess.run(['import','-display',display,'-window',host_window,str(out/'nested-minimized.png')],env=host_env,check=True)
        restored=nested_kwin('if (w.pid !== '+str(app.pid)+') throw new Error("foreign window"); w.minimized=false; workspace.activeWindow=w; ')
        await asyncio.sleep(.3)
        restored_readback=nested_kwin()
        assert restored_readback['internalId']==before['internalId'] and restored_readback['minimized'] is False and restored_readback['active'] is True,restored_readback
        subprocess.run(['import','-display',display,'-window',host_window,str(out/'nested-restored.png')],env=host_env,check=True)
        assert await probe.js('!!document.querySelector("#btn-win-min")')
        report={'binarySha256':hashlib.sha256(Path('target/debug/rustrss-desktop').read_bytes()).hexdigest(),'environment':'private Xvfb host + private dbus-run-session nested KWin X11 backend + owned Wayland RustRss; isolated HOME/XDG_DATA_HOME/XDG_CONFIG_HOME/XDG_CACHE_HOME/DB','hostDisplay':display,'hostWindow':host_window,'ownedAppPid':app.pid,'pointer':{'method':'XTest on host Xvfb, forwarded by nested KWin','root':[rx.value,ry.value],'target':[target_x,target_y],'buttonCss':button,'clicks':clicks},'kwinBefore':before,'kwinMinimized':after,'kwinRestoreCommand':restored,'kwinRestored':restored_readback,'screenshots':['nested-before.png','nested-minimized.png','nested-restored.png'],'passed':True,'limit':'No taskbar shell in nested compositor; restoration uses PID-guarded KWin compositor state.'}
        (out/'nested-window-results.json').write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps({'passed':True,'trustedClick':clicks[-1]['trusted'],'minimized':after['minimized'],'restored':not restored_readback['minimized']}),flush=True)
    finally:
        for name in scripts:
            subprocess.run(['qdbus6','org.kde.KWin','/Scripting','org.kde.kwin.Scripting.unloadScript',name],env=host_env,capture_output=True,text=True)
        if app and app.poll() is None:app.terminate();app.wait(timeout=10)
        if kwin and kwin.poll() is None:kwin.terminate();kwin.wait(timeout=10)
        if xvfb.poll() is None:xvfb.terminate();xvfb.wait(timeout=10)

if __name__=='__main__':
    if not os.environ.get('RUSTSS_T6_NESTED_ROOT'):
        private_root=Path(tempfile.mkdtemp(prefix='rustrss-t6-nested-'))
        (private_root/'home').mkdir();(private_root/'runtime').mkdir(mode=0o700)
        private_env=dict(os.environ,HOME=str(private_root/'home'),XDG_DATA_HOME=str(private_root/'data'),XDG_CONFIG_HOME=str(private_root/'config'),XDG_CACHE_HOME=str(private_root/'cache'),XDG_RUNTIME_DIR=str(private_root/'runtime'),RUSTSS_T6_NESTED_ROOT=str(private_root))
        private_env.pop('DBUS_SESSION_BUS_ADDRESS',None)
        os.execvpe('dbus-run-session',['dbus-run-session','--',sys.executable,*sys.argv],private_env)
    asyncio.run(main())
