"""Observe the owned desktop window at the KWin compositor after native IPC.

Runs on the current real KDE session with an isolated HOME/XDG_DATA_HOME/DB.
The temporary KWin scripts read or restore only the launched process PID.
"""
import asyncio
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
key_spec=importlib.util.spec_from_file_location('keyboard_probe',Path(__file__).with_name('verify-reading-experience-t6-desktop-input.py'))
keyboard_probe=importlib.util.module_from_spec(key_spec);key_spec.loader.exec_module(keyboard_probe)

def run(*args):return subprocess.check_output(args,text=True,timeout=15).strip()
def free_port():
    with socket.socket() as s:s.bind(('127.0.0.1',0));return s.getsockname()[1]

async def main():
    out=Path(sys.argv[1]).resolve();out.mkdir(parents=True,exist_ok=True)
    assert os.environ.get('WAYLAND_DISPLAY') and os.environ.get('DBUS_SESSION_BUS_ADDRESS')
    backend='x11' if '--x11' in sys.argv[2:] else 'wayland'
    root=Path(tempfile.mkdtemp(prefix='rustrss-t6-compositor-'));(root/'home').mkdir()
    db=root/'fixture.sqlite';shutil.copy2('/tmp/rustrss-t4-fixture.sqlite',db)
    env=dict(os.environ,HOME=str(root/'home'),XDG_DATA_HOME=str(root/'data'),RUSTSS_DB=str(db),GDK_BACKEND=backend,GDK_GL='disable',WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{free_port()}')
    if backend=='x11':env.pop('WAYLAND_DISPLAY',None)
    else:env.pop('DISPLAY',None)
    app=subprocess.Popen(['target/debug/rustrss-desktop'],env=env,stdout=(out/('compositor-'+backend+'-app.log')).open('w'),stderr=subprocess.STDOUT)
    scripts=[]
    def kwin(action=''):
        tag='T6OWN'+uuid.uuid4().hex
        path=root/(tag+'.js')
        payload='for (const w of workspace.windowList()) if (w.pid === '+str(app.pid)+') { try { '+action+' print("'+tag+' "+JSON.stringify({pid:w.pid,internalId:w.internalId,minimized:w.minimized,active:workspace.activeWindow?.internalId===w.internalId,skipTaskbar:w.skipTaskbar,frame:w.frameGeometry})); } catch (e) { print("'+tag+' "+JSON.stringify({error:String(e),pid:w.pid})); } }'
        path.write_text(payload+'\n')
        since=str(int(time.time())-1)
        sid=run('qdbus6','org.kde.KWin','/Scripting','org.kde.kwin.Scripting.loadScript',str(path),tag)
        scripts.append(tag)
        run('qdbus6','org.kde.KWin','/Scripting/Script'+sid,'org.kde.kwin.Script.run')
        for _ in range(30):
            journal=subprocess.run(['journalctl','--user','--since','@'+since,'-g',tag,'-o','cat','--no-pager'],capture_output=True,text=True,timeout=10)
            lines=[line.split(tag+' ',1)[1] for line in journal.stdout.splitlines() if tag+' ' in line]
            if lines:
                result=json.loads(lines[-1]);assert not result.get('error'),result
                return result
            time.sleep(.1)
        raise AssertionError('KWin did not report owned PID '+str(app.pid))
    def candidates():
        tag='T6CAND'+uuid.uuid4().hex
        path=root/(tag+'.js')
        path.write_text('for (const w of workspace.windowList()) if (String(w.resourceClass)==="rustrss-desktop") print("'+tag+' "+JSON.stringify({pid:w.pid,internalId:w.internalId,resourceClass:w.resourceClass}));\n')
        since=str(int(time.time())-1)
        sid=run('qdbus6','org.kde.KWin','/Scripting','org.kde.kwin.Scripting.loadScript',str(path),tag);scripts.append(tag)
        run('qdbus6','org.kde.KWin','/Scripting/Script'+sid,'org.kde.kwin.Script.run')
        time.sleep(.3)
        journal=subprocess.run(['journalctl','--user','--since','@'+since,'-g',tag,'-o','cat','--no-pager'],capture_output=True,text=True,timeout=10)
        return [json.loads(line.split(tag+' ',1)[1]) for line in journal.stdout.splitlines() if tag+' ' in line]
    try:
        probe=native_probe.Probe({'root':str(root),'inspector_port':int(env['WEBKIT_INSPECTOR_HTTP_SERVER'].split(':')[-1])})
        for _ in range(200):
            try:await probe.js('1');break
            except (OSError,IndexError):assert app.poll() is None;await asyncio.sleep(.1)
        await probe.until('!!document.querySelector("#btn-win-min")')
        await asyncio.sleep(1.5)
        try:before=kwin('workspace.activeWindow=w; ')
        except AssertionError as error:raise AssertionError({'error':str(error),'candidates':candidates(),'ownPid':app.pid})
        assert before['pid']==app.pid and before['minimized'] is False and before['active'] is True,before
        if backend=='x11' and '--trusted-x11' in sys.argv[2:]:
            listing=run('xwininfo','-root','-tree')
            matches=re.findall(r'(0x[0-9a-f]+) "RustRss"',listing)
            owned=[]
            for ident in matches:
                prop=run('xprop','-id',ident,'_NET_WM_PID')
                if re.search(r'=\s*'+str(app.pid)+r'\b',prop):owned.append(ident)
            assert len(owned)==1,{'owned':owned,'pid':app.pid}
            await probe.js('window.__T6_TITLE_CLICKS=[];document.querySelector("#btn-win-min").addEventListener("click",e=>window.__T6_TITLE_CLICKS.push({trusted:e.isTrusted,detail:e.detail,target:e.target.id}),true);document.querySelector("#btn-win-min").focus();true')
            assert await probe.js('document.activeElement?.id')=='btn-win-min'
            keyboard_probe.XKeyboard(os.environ['DISPLAY'],int(owned[0],16)).key('Return')
            await probe.until('window.__T6_TITLE_CLICKS.length>0')
            clicks=await probe.js('window.__T6_TITLE_CLICKS')
            assert clicks[-1]['trusted'] and clicks[-1]['target']=='btn-win-min',clicks
            ipc='titlebar handler from trusted XTest Return'
        else:
            await probe.js('window.__T6_MIN="pending";window.__TAURI__.core.invoke("window_minimize").then(()=>window.__T6_MIN="ok",e=>window.__T6_MIN=String(e));true')
            await probe.until('window.__T6_MIN!=="pending"')
            ipc=await probe.js('window.__T6_MIN');clicks=[]
        await asyncio.sleep(.6)
        after=kwin()
        restored=kwin('if (w.pid !== '+str(app.pid)+') throw new Error("foreign window"); w.minimized=false; workspace.activeWindow=w; ')
        await asyncio.sleep(.3)
        restore_readback=kwin()
        report={'binarySha256':hashlib.sha256(Path('target/debug/rustrss-desktop').read_bytes()).hexdigest(),'backend':backend,'session':'real KWin with isolated owned app HOME/XDG_DATA_HOME/DB','pid':app.pid,'input':'trusted XTest Return on focused titlebar minimize button' if backend=='x11' and '--trusted-x11' in sys.argv[2:] else 'direct window_minimize IPC','ipc':ipc,'trustedClicks':clicks,'before':before,'after':after,'restoreCommand':restored,'restored':restore_readback,'windowMinimized':after['minimized'] is True,'restoredVisible':restore_readback['minimized'] is False and restore_readback['active'] is True}
        (out/('compositor-'+backend+'-results.json')).write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps({'backend':backend,'ipc':ipc,'minimized':after['minimized'],'restored':report['restoredVisible']}))
    finally:
        for name in scripts:
            subprocess.run(['qdbus6','org.kde.KWin','/Scripting','org.kde.kwin.Scripting.unloadScript',name],capture_output=True,text=True)
        if app.poll() is None:app.terminate();app.wait(timeout=10)

if __name__=='__main__':asyncio.run(main())
