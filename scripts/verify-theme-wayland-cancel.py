"""Finish the open T7 native-input check after verify-theme-wayland.py.
Default: wait for a human click. --portal: ask ONCE for KDE pointer permission,
then target only the isolated preview. Requires GI, websockets, qdbus6, journalctl.
This follow-up was syntax checked, not runtime accepted in T7 (user absent).
"""
import argparse
import asyncio
import json
from pathlib import Path
import re
import sqlite3
import subprocess
import time
import urllib.request
import uuid
import websockets

parser = argparse.ArgumentParser()
parser.add_argument('instance')
parser.add_argument('--portal', action='store_true')
args = parser.parse_args()
info = json.loads(Path(args.instance).read_text())
root = Path(info['root'])
previous = json.loads((root/'wayland-results.json').read_text())
candidate = previous['local_cancel_candidate']
click_marker = 'T7_LOCAL_CANCEL_' + uuid.uuid4().hex

async def evaluate(index, expression):
    with urllib.request.urlopen(f"http://127.0.0.1:{info['inspector_port']}/", timeout=5) as r:
        paths = re.findall(r'(/socket/\d+/\d+/WebPage)', r.read().decode())
    async with websockets.connect(f"ws://127.0.0.1:{info['inspector_port']}"+paths[index]) as ws:
        while True:
            event = json.loads(await asyncio.wait_for(ws.recv(), 10))
            if event.get('method') == 'Target.targetCreated' and event['params']['targetInfo']['type'] == 'page':
                target = event['params']['targetInfo']['targetId']; break
        inner = {'id':1,'method':'Runtime.evaluate','params':{'expression':expression,'returnByValue':True}}
        await ws.send(json.dumps({'id':1,'method':'Target.sendMessageToTarget','params':{'targetId':target,'message':json.dumps(inner)}}))
        while True:
            msg = json.loads(await asyncio.wait_for(ws.recv(), 10))
            if msg.get('method') != 'Target.dispatchMessageFromTarget': continue
            result = json.loads(msg['params']['message'])
            if result.get('id') != 1: continue
            assert 'error' not in result and not result['result'].get('wasThrown'), result
            return result['result']['result'].get('value')

def js(index, expression):
    return asyncio.run(evaluate(index, expression))

def stored():
    with sqlite3.connect(root/'fixture.sqlite') as db:
        return db.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone()[0]

# Reject stale/cross-profile requests before touching the pointer.
assert Path(f"/proc/{info['pid']}").exists()
assert candidate['capture']['display_backend'] == 'GdkWaylandDisplay'
assert json.loads(stored())['current']['revision'] == candidate['base_revision']
formal = stored()
geometry = json.loads(js(1, """JSON.stringify((()=>{
 const b=document.getElementById('preview-cancel'), old=b.onclick, r=b.getBoundingClientRect();
 b.onclick=async e=>{await __TAURI__.core.invoke('ui_log',{line:'__CLICK_MARKER__ '+JSON.stringify({trusted:e.isTrusted,type:e.type})});return old.call(b,e);};
 return {x:r.x,y:r.y,width:r.width,height:r.height,viewport:[innerWidth,innerHeight]};})())""".replace("__CLICK_MARKER__", click_marker)))

session = None
bus = None
try:
    if args.portal:
        from gi.repository import Gio, GLib
        bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        responses = {}
        bus.signal_subscribe(None,'org.freedesktop.portal.Request','Response',None,None,Gio.DBusSignalFlags.NONE,lambda c,s,p,i,n,a: responses.update({p:a.unpack()}))
        def call(method, signature, values):
            return bus.call_sync('org.freedesktop.portal.Desktop','/org/freedesktop/portal/desktop','org.freedesktop.portal.RemoteDesktop',method,GLib.Variant(signature,values),None,Gio.DBusCallFlags.NONE,10000,None).unpack()
        def request(method, signature, values):
            path = call(method, signature, values)[0]
            deadline = time.monotonic()+120
            while path not in responses:
                if time.monotonic()>deadline: raise TimeoutError('Authorization timed out; no retry')
                while GLib.MainContext.default().pending(): GLib.MainContext.default().iteration(False)
                time.sleep(.05)
            status, value = responses[path]
            assert status == 0, (method,status)
            return value
        session = request('CreateSession','(a{sv})',({'session_handle_token':GLib.Variant('s','rustrsst7')},))['session_handle']
        request('SelectDevices','(oa{sv})',(session,{'types':GLib.Variant('u',2)}))
        print('Allow the single KDE pointer authorization prompt (120s); no automatic retry.', flush=True)
        grant = request('Start','(osa{sv})',(session,'',{}))
        assert grant['devices'] & 2
        def kwin_position():
            tag = 'T7_POINTER_'+uuid.uuid4().hex
            script = root/'pointer-position.js'
            script.write_text("for(const w of workspace.windowList()) if(w.pid==="+str(int(info['pid']))+" && w.caption==='RustRss — Theme preview'){workspace.activeWindow=w;print('"+tag+" '+JSON.stringify({frame:w.frameGeometry,cursor:workspace.cursorPos}));}")
            name = 'rustrss-t7-'+uuid.uuid4().hex
            sid = subprocess.check_output(['qdbus6','org.kde.KWin','/Scripting','org.kde.kwin.Scripting.loadScript',str(script),name],text=True).strip()
            try:
                subprocess.run(['qdbus6','org.kde.KWin','/Scripting/Script'+sid,'org.kde.kwin.Script.run'],check=True)
                time.sleep(.3)
                logs = subprocess.check_output(['journalctl','--user','-n','100','-o','cat'],text=True)
                matches = [json.loads(line.split(tag+' ',1)[1]) for line in logs.splitlines() if tag+' ' in line]
                assert len(matches)==1, 'Expected exactly one isolated preview window'
                return matches[0]
            finally:
                subprocess.run(['qdbus6','org.kde.KWin','/Scripting','org.kde.kwin.Scripting.unloadScript',name],check=True,stdout=subprocess.DEVNULL)
        pos = kwin_position()
        frame = pos['frame']
        # Fail closed if decorations/coordinate scaling require a different mapping.
        assert [frame['width'],frame['height']] == geometry['viewport'], 'Frame/content mismatch; use manual click'
        x = frame['x']+geometry['x']+geometry['width']/2
        y = frame['y']+geometry['y']+geometry['height']/2
        call('NotifyPointerMotion','(oa{sv}dd)',(session,{},x-pos['cursor']['x'],y-pos['cursor']['y']))
        time.sleep(.3)
        moved = kwin_position()
        assert moved['frame']==frame and abs(moved['cursor']['x']-x)<3 and abs(moved['cursor']['y']-y)<3, 'Pointer target not reached; no click sent'
        call('NotifyPointerButton','(oa{sv}iu)',(session,{},272,1))
        time.sleep(.1)
        call('NotifyPointerButton','(oa{sv}iu)',(session,{},272,0))
    else:
        print('Click the isolated RustRss theme preview cancel button now (120s).', flush=True)
    deadline = time.monotonic()+120
    click = None
    while time.monotonic()<deadline:
        for log in root.glob('desktop*.log'):
            for line in log.read_text().splitlines():
                if click_marker+' ' in line:
                    click = json.JSONDecoder().raw_decode(line.split(click_marker+' ',1)[1])[0]
        if click: break
        time.sleep(.2)
    assert click and click['trusted'], 'No trusted native click observed'
    time.sleep(.5)
    measurement = json.loads(js(0,"JSON.stringify({offset:__t7.paragraph.getBoundingClientRect().top-__t7.reader.getBoundingClientRect().top,sameArticle:__t7.article===document.querySelector('#reader .article'),sameRow:__t7.row===document.querySelector('#entries li.active'),sameParagraph:__t7.paragraph.isConnected})"))
    assert measurement['sameArticle'] and measurement['sameRow'] and measurement['sameParagraph']
    assert abs(measurement['offset']-previous['before_cancel']['offset'])<2
    assert stored()==formal
    payload = {'jsonrpc':'2.0','id':1,'method':'tools/call','params':{'name':'finish_theme_preview','arguments':{'preview_id':candidate['preview_id'],'expected_preview_revision':candidate['expected_preview_revision'],'action':'save'}}}
    req = urllib.request.Request(f"http://127.0.0.1:{info['mcp_port']}/mcp",data=json.dumps(payload).encode(),headers={'Authorization':'Bearer fixture-write','Content-Type':'application/json','Accept':'application/json, text/event-stream'})
    with urllib.request.urlopen(req,timeout=15) as r: result = json.load(r)['result']
    assert result.get('isError'), result
    assert stored()==formal
    with urllib.request.urlopen(f"http://127.0.0.1:{info['inspector_port']}/",timeout=5) as r:
        assert len(re.findall(r'(/socket/\d+/\d+/WebPage)',r.read().decode()))==1, 'Preview still alive'
    report = {'trusted_click':click,'reader':measurement,'formal_unchanged':True,'preview_destroyed':True,'save_rejected':result.get('structuredContent')}
    (root/'native-cancel-results.json').write_text(json.dumps(report,indent=2))
    print(json.dumps(report))
finally:
    if session and bus:
        bus.call_sync('org.freedesktop.portal.Desktop',session,'org.freedesktop.portal.Session','Close',None,None,Gio.DBusCallFlags.NONE,10000,None)
