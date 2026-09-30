"""Trusted XTest keyboard check in an isolated Xvfb desktop session."""
import asyncio
import ctypes
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native_probe)

def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]

class XKeyboard:
    def __init__(self, display, window):
        self.x11 = ctypes.CDLL('libX11.so.6')
        self.xtst = ctypes.CDLL('libXtst.so.6')
        self.x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self.x11.XOpenDisplay.restype = ctypes.c_void_p
        self.x11.XStringToKeysym.argtypes = [ctypes.c_char_p]
        self.x11.XStringToKeysym.restype = ctypes.c_ulong
        self.x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        self.x11.XKeysymToKeycode.restype = ctypes.c_ubyte
        self.x11.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
        self.x11.XFlush.argtypes = [ctypes.c_void_p]
        self.x11.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
        self.x11.XGetInputFocus.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ulong), ctypes.POINTER(ctypes.c_int)]
        self.xtst.XTestFakeKeyEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        self.display = self.x11.XOpenDisplay(display.encode())
        assert self.display, f'XOpenDisplay failed: {display}'
        self.window = window

    def key(self, name, shift=False):
        self.x11.XSetInputFocus(self.display, self.window, 2, 0)
        self.x11.XSync(self.display, 0)
        focused = ctypes.c_ulong()
        revert = ctypes.c_int()
        self.x11.XGetInputFocus(self.display, ctypes.byref(focused), ctypes.byref(revert))
        assert focused.value == self.window, 'X keyboard focus did not reach the app window'
        names = (['Shift_L'] if shift else []) + [name]
        codes = []
        for item in names:
            sym = self.x11.XStringToKeysym(item.encode())
            code = self.x11.XKeysymToKeycode(self.display, sym)
            assert code, f'No keycode for {item}'
            codes.append(code)
            assert self.xtst.XTestFakeKeyEvent(self.display, code, 1, 0)
            self.x11.XSync(self.display, 0)
            time.sleep(.08)
        for code in reversed(codes):
            assert self.xtst.XTestFakeKeyEvent(self.display, code, 0, 0)
        self.x11.XSync(self.display, 0)
        time.sleep(.15)

async def main():
    out = Path(sys.argv[1]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix='rustrss-t6-desktop-input-'))
    runtime = root / 'runtime'
    runtime.mkdir(mode=0o700)
    (root / 'home').mkdir()
    db = root / 'fixture.sqlite'
    shutil.copy2('/tmp/rustrss-t4-fixture.sqlite', db)
    env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
               XDG_RUNTIME_DIR=str(runtime), RUSTSS_DB=str(db), GDK_GL='disable',
               WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{free_port()}')
    env.pop('DISPLAY', None)
    env.pop('WAYLAND_DISPLAY', None)
    old_sockets = set(Path('/tmp/.X11-unix').glob('X*'))
    display = next(':' + str(i) for i in range(70, 90) if Path(f'/tmp/.X11-unix/X{i}') not in old_sockets)
    xvfb_binary = os.environ.get('T4_XVFB') or shutil.which('Xvfb') or '/tmp/rustrss-t4-xvfb/root/usr/bin/Xvfb'
    assert Path(xvfb_binary).exists(), f'Xvfb not found: {xvfb_binary}'
    with (out / 'review-desktop-xvfb.log').open('w') as log:
        xvfb = subprocess.Popen([xvfb_binary, display,
                                 '-screen', '0', '1600x1000x24', '-nolisten', 'tcp', '-ac'],
                                env=env, stdout=log, stderr=subprocess.STDOUT)
    app = None
    try:
        for _ in range(200):
            assert xvfb.poll() is None, 'Xvfb exited'
            if Path('/tmp/.X11-unix/X' + display[1:]).exists(): break
            await asyncio.sleep(.1)
        assert Path('/tmp/.X11-unix/X' + display[1:]).exists(), 'Isolated Xvfb display did not appear'
        env.update(DISPLAY=display, GDK_BACKEND='x11')
        with (out / 'review-desktop-app.log').open('w') as log:
            app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log)
        port = int(env['WEBKIT_INSPECTOR_HTTP_SERVER'].split(':')[-1])
        probe = native_probe.Probe({'root': str(root), 'inspector_port': port})
        for _ in range(200):
            try:
                await probe.js('1')
                break
            except (OSError, IndexError):
                assert app.poll() is None, 'App exited'
                await asyncio.sleep(.1)
        await probe.until('!!document.querySelector("#entries li[data-id=\\"1\\"] .title")')
        listing = subprocess.check_output(['xwininfo', '-root', '-tree'], env=env, text=True)
        (out / 'review-desktop-windows.txt').write_text(listing.rstrip() + '\n')
        found = re.search(r'(0x[0-9a-f]+) "RustRss"', listing)
        assert found, 'RustRss X11 window not found'
        keyboard = XKeyboard(display, int(found.group(1), 16))
        checks = []
        def check(condition, description):
            assert condition, description
            checks.append(description)
        await probe.js('window.__T4_KEYS=[];document.addEventListener("keydown",e=>window.__T4_KEYS.push({key:e.key,trusted:e.isTrusted}),true)')
        await probe.js('document.querySelector("#entries li[data-id=\\"1\\"] .title").click()')
        await probe.until('!!document.querySelector("#act-more")')
        await probe.js('document.querySelector("#act-more").click()')
        await probe.until('!!document.querySelector("#ctx-menu")')
        check(await probe.js('document.activeElement===document.querySelector("#ctx-menu > button")'), 'More opens with first item focused')
        with sqlite3.connect(db) as conn:
            before = conn.execute('SELECT read FROM entries WHERE id=1').fetchone()[0]
        check(before == 1, 'opened entry stored as read')
        keyboard.key('Tab')
        check(await probe.js('document.activeElement===document.querySelectorAll("#ctx-menu > button")[1]'), 'trusted Tab moves focus')
        keyboard.key('Tab', shift=True)
        check(await probe.js('document.activeElement===document.querySelector("#ctx-menu > button")'), 'trusted Shift+Tab restores first item')
        keyboard.key('Escape')
        await probe.until('!document.querySelector("#ctx-menu")')
        check(await probe.js('document.activeElement?.id==="act-more"'), 'trusted Escape closes and restores More focus')
        with sqlite3.connect(db) as conn:
            check(conn.execute('SELECT read FROM entries WHERE id=1').fetchone()[0] == before, 'Escape cancellation causes zero write')
        await probe.js('document.querySelector("#act-more").click()')
        await probe.until('!!document.querySelector("#ctx-menu")')
        await probe.js('[...document.querySelectorAll("#ctx-menu button")].find(n=>n.textContent.includes("标为未读")).focus()')
        check(await probe.js('document.activeElement.textContent.includes("标为未读")'), 'mark unread has keyboard focus')
        keyboard.key('Return')
        await probe.until('!document.querySelector("#ctx-menu")')
        with sqlite3.connect(db) as conn:
            after = conn.execute('SELECT read FROM entries WHERE id=1').fetchone()[0]
        check(after == 0, 'trusted Enter activates mark unread once')
        await probe.js('document.activeElement?.blur();true')
        keyboard.key('slash',shift=True)
        check(await probe.js('document.querySelector("#keyboard-help").open'), 'trusted ? opens shortcut help')
        keyboard.key('Escape')
        check(await probe.js('!document.querySelector("#keyboard-help").open'), 'trusted Escape closes shortcut help')
        keyboard.key('slash')
        check(await probe.js('document.activeElement?.id==="search"'), 'trusted / focuses search')
        keyboard.key('Escape')
        check(await probe.js('document.querySelector("#search").value===""'), 'trusted Escape clears search')
        await probe.js('document.activeElement?.blur();document.querySelector("#btn-settings").focus();document.querySelector("#btn-settings").click();true')
        await probe.until('!document.querySelector("#settings-overlay").classList.contains("hidden")')
        check(await probe.js('document.querySelector("#settings-overlay").getAttribute("aria-modal")==="true" && document.activeElement?.id==="tab-appearance"'), 'desktop settings opens as modal with tab focus')
        keyboard.key('Tab')
        check(await probe.js('document.activeElement?.dataset.themeTop==="mode"'), 'trusted Tab enters active settings pane')
        keyboard.key('Tab',shift=True)
        check(await probe.js('document.activeElement?.id==="tab-appearance"'), 'trusted Shift+Tab reverses settings focus')
        keyboard.key('Escape')
        check(await probe.js('document.querySelector("#settings-overlay").classList.contains("hidden") && document.activeElement?.id==="btn-settings"'), 'trusted Escape closes settings and restores trigger focus')
        point=await probe.js('''(() => {const n=document.querySelector('#feeds li[data-feed-id] .name');n.scrollIntoView({block:'center'});const r=n.getBoundingClientRect();return {x:Math.round(r.x+r.width/2),y:Math.round(r.y+r.height/2),height:innerHeight}})()''')
        check(0<point['y']<point['height'],'feed row in viewport for native right-click')
        keyboard.xtst.XTestFakeMotionEvent.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.c_int,ctypes.c_int,ctypes.c_ulong]
        keyboard.xtst.XTestFakeButtonEvent.argtypes=[ctypes.c_void_p,ctypes.c_uint,ctypes.c_int,ctypes.c_ulong]
        keyboard.xtst.XTestFakeMotionEvent(keyboard.display,-1,point['x'],point['y'],0)
        keyboard.xtst.XTestFakeButtonEvent(keyboard.display,3,1,0)
        keyboard.xtst.XTestFakeButtonEvent(keyboard.display,3,0,0)
        keyboard.x11.XSync(keyboard.display,0)
        await probe.until('!!document.querySelector("#ctx-menu")')
        check(await probe.js('document.querySelector("#ctx-menu").getAttribute("role")==="menu"'), 'native right-click opens anchored feed menu')
        keyboard.key('Escape')
        check(await probe.js('!document.querySelector("#ctx-menu")'), 'trusted Escape closes native right-click menu')
        # Native list, reader and tag shortcuts. These read UI selection and the
        # same isolated SQLite rows rather than just observing keydown events.
        await probe.js('document.querySelector("#views li[data-kind=all]").click();true')
        await probe.until('document.querySelectorAll("#entries li[data-id]").length===30')
        ids=await probe.js('[...document.querySelectorAll("#entries li[data-id]")].map(n=>Number(n.dataset.id))')
        async def selected():
            return await probe.js('Number(document.querySelector("#entries li.active")?.dataset.id)')
        def flags(entry_id):
            with sqlite3.connect(db) as conn:
                return conn.execute('SELECT read,starred,read_later FROM entries WHERE id=?',(entry_id,)).fetchone()
        keyboard.key('g')
        await probe.until(f'document.querySelector("#entries li.active")?.dataset.id==="{ids[0]}"')
        check(await selected()==ids[0],'trusted g selects first article')
        keyboard.key('j')
        await probe.until(f'document.querySelector("#entries li.active")?.dataset.id==="{ids[1]}"')
        check(await selected()==ids[1] and await probe.js('!!document.querySelector("#reader .article")'),'trusted j moves list and reader to next article')
        keyboard.key('k')
        await probe.until(f'document.querySelector("#entries li.active")?.dataset.id==="{ids[0]}"')
        check(await selected()==ids[0],'trusted k moves to previous article')
        keyboard.key('Down')
        await probe.until(f'document.querySelector("#entries li.active")?.dataset.id==="{ids[1]}"')
        check(await selected()==ids[1],'trusted ArrowDown moves list')
        keyboard.key('Up')
        await probe.until(f'document.querySelector("#entries li.active")?.dataset.id==="{ids[0]}"')
        check(await selected()==ids[0],'trusted ArrowUp moves list')
        keyboard.key('g',shift=True)
        await probe.until(f'document.querySelector("#entries li.active")?.dataset.id==="{ids[-1]}"')
        check(await selected()==ids[-1],'trusted G selects last article')
        keyboard.key('g')
        await probe.until(f'document.querySelector("#entries li.active")?.dataset.id==="{ids[0]}"')
        check(await selected()==ids[0],'trusted g returns to first article')
        keyboard.key('Return')
        check(await selected()==ids[0] and await probe.js('!!document.querySelector("#reader .article")'),'trusted Enter opens selected article')
        selected_id=ids[0]
        old_flags=flags(selected_id)
        keyboard.key('u')
        for _ in range(50):
            if flags(selected_id)[0]!=old_flags[0]:break
            await asyncio.sleep(.1)
        check(flags(selected_id)[0]==1-old_flags[0],'trusted u toggles selected article read state in SQLite')
        keyboard.key('s')
        for _ in range(50):
            if flags(selected_id)[1]!=old_flags[1]:break
            await asyncio.sleep(.1)
        check(flags(selected_id)[1]==1-old_flags[1],'trusted s toggles star in SQLite')
        keyboard.key('l')
        for _ in range(50):
            if flags(selected_id)[2]!=old_flags[2]:break
            await asyncio.sleep(.1)
        check(flags(selected_id)[2]==1-old_flags[2],'trusted l toggles read later in SQLite')
        keyboard.key('t')
        await probe.until('!document.querySelector("#tag-picker-overlay").classList.contains("hidden")')
        check(await probe.js('document.activeElement?.id==="tag-picker-input"'),'trusted t opens tag picker with input focus')
        picker_flags=flags(selected_id)
        keyboard.key('Escape')
        check(await probe.js('document.querySelector("#tag-picker-overlay").classList.contains("hidden")') and flags(selected_id)==picker_flags,'trusted Escape closes tag picker without article write')
        with sqlite3.connect(db) as conn: old_unread=conn.execute("SELECT value FROM settings WHERE key='list.hide_read'").fetchone()
        keyboard.key('u',shift=True)
        for _ in range(50):
            with sqlite3.connect(db) as conn: new_unread=conn.execute("SELECT value FROM settings WHERE key='list.hide_read'").fetchone()
            if new_unread!=old_unread:break
            await asyncio.sleep(.1)
        check(new_unread!=old_unread,'trusted U toggles stored unread-only setting')
        await probe.until('document.querySelector("#btn-unread-only").getAttribute("aria-pressed")==="true"')
        keyboard.key('u',shift=True)
        for _ in range(50):
            with sqlite3.connect(db) as conn: reset_unread=conn.execute("SELECT value FROM settings WHERE key='list.hide_read'").fetchone()
            if reset_unread==old_unread:break
            await asyncio.sleep(.1)
        check(reset_unread==('false',),'trusted U restores effective unread-only=false')
        await probe.until('document.querySelector("#btn-unread-only").getAttribute("aria-pressed")==="false"')
        with sqlite3.connect(db) as conn: unread_before=conn.execute('SELECT COUNT(*) FROM entries WHERE read=0').fetchone()[0]
        check(unread_before>0,'bulk shortcut has unread synthetic rows')
        keyboard.key('a',shift=True)
        for _ in range(100):
            with sqlite3.connect(db) as conn: unread_after=conn.execute('SELECT COUNT(*) FROM entries WHERE read=0').fetchone()[0]
            if unread_after==0:break
            await asyncio.sleep(.1)
        check(unread_after==0,'trusted A marks current All view read in SQLite')
        await probe.js('document.querySelector("#search").focus();true')
        suppress_flags=flags(selected_id)
        keyboard.key('s')
        check(await probe.js('document.querySelector("#search").value==="s"') and flags(selected_id)==suppress_flags,'typing in search suppresses star shortcut')
        keyboard.key('Escape')
        await probe.js('document.querySelector("#btn-settings").click();true')
        await probe.until('!document.querySelector("#settings-overlay").classList.contains("hidden")')
        modal_selection=await selected();modal_flags=flags(selected_id)
        keyboard.key('j');keyboard.key('s');keyboard.key('l');keyboard.key('t')
        check(await selected()==modal_selection and flags(selected_id)==modal_flags and await probe.js('document.querySelector("#tag-picker-overlay").classList.contains("hidden")'),'settings modal suppresses list, reader and tag shortcuts')
        keyboard.key('Escape')
        await probe.until('document.querySelector("#settings-overlay").classList.contains("hidden")')
        await probe.js('window.__T6_REFRESH=[];new MutationObserver(()=>window.__T6_REFRESH.push(document.querySelector("#status").textContent)).observe(document.querySelector("#status"),{childList:true,subtree:true,characterData:true});true')
        keyboard.key('r')
        await probe.until('window.__T6_REFRESH.some(s=>s.includes("正在刷新"))')
        refresh_statuses=await probe.js('window.__T6_REFRESH')
        check(any('正在刷新' in s for s in refresh_statuses),'trusted r starts refresh feedback')
        events = await probe.js('window.__T4_KEYS')
        check(all(e['trusted'] for e in events) and {'Tab', 'Escape', 'Enter','j','k','u','s','l','t','U','A','r','g','G'} <= {e['key'] for e in events}, 'list, reader, tag and modal keyboard events are trusted browser input')
        button=await probe.js('''(() => {const el=document.querySelector("#btn-win-min");const r=el.getBoundingClientRect();window.__T6_MIN_CLICK=[];el.addEventListener("click",e=>window.__T6_MIN_CLICK.push({trusted:e.isTrusted,detail:e.detail,currentTarget:e.currentTarget.id}),true);return {x:r.x+r.width/2,y:r.y+r.height/2,dpr:devicePixelRatio}})()''')
        geometry=subprocess.check_output(['xwininfo','-id',found.group(1)],env=env,text=True)
        origin_x=int(re.search(r'Absolute upper-left X:\s*(-?\d+)',geometry).group(1))
        origin_y=int(re.search(r'Absolute upper-left Y:\s*(-?\d+)',geometry).group(1))
        px=round(origin_x+button['x']*button['dpr']);py=round(origin_y+button['y']*button['dpr'])
        keyboard.xtst.XTestFakeMotionEvent.argtypes=[ctypes.c_void_p,ctypes.c_int,ctypes.c_int,ctypes.c_int,ctypes.c_ulong]
        keyboard.xtst.XTestFakeButtonEvent.argtypes=[ctypes.c_void_p,ctypes.c_uint,ctypes.c_int,ctypes.c_ulong]
        keyboard.x11.XQueryPointer.argtypes=[ctypes.c_void_p,ctypes.c_ulong,ctypes.POINTER(ctypes.c_ulong),ctypes.POINTER(ctypes.c_ulong),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_int),ctypes.POINTER(ctypes.c_uint)]
        check(keyboard.xtst.XTestFakeMotionEvent(keyboard.display,-1,px,py,0),'private Xvfb XTest pointer moved toward owned titlebar')
        keyboard.x11.XSync(keyboard.display,0)
        root_return=ctypes.c_ulong();child_return=ctypes.c_ulong();rx=ctypes.c_int();ry=ctypes.c_int();wx=ctypes.c_int();wy=ctypes.c_int();mask=ctypes.c_uint()
        keyboard.x11.XQueryPointer(keyboard.display,int(found.group(1),16),ctypes.byref(root_return),ctypes.byref(child_return),ctypes.byref(rx),ctypes.byref(ry),ctypes.byref(wx),ctypes.byref(wy),ctypes.byref(mask))
        check(abs(rx.value-px)<=2 and abs(ry.value-py)<=2,'private Xvfb pointer verified over owned titlebar button')
        assert keyboard.xtst.XTestFakeButtonEvent(keyboard.display,1,1,0)
        assert keyboard.xtst.XTestFakeButtonEvent(keyboard.display,1,0,0)
        keyboard.x11.XSync(keyboard.display,0)
        await probe.until('window.__T6_MIN_CLICK.length>0')
        titlebar_clicks=await probe.js('window.__T6_MIN_CLICK')
        assert titlebar_clicks[-1]['trusted'] and titlebar_clicks[-1]['currentTarget']=='btn-win-min',{'clicks':titlebar_clicks,'button':button,'root':[rx.value,ry.value],'target':[px,py]}
        check(True,'private Xvfb native pointer activates actual minimize button')
        report = {'display': display, 'source': 'XTestFakeKeyEvent and XTestFakeButtonEvent on isolated Xvfb', 'window': found.group(1),
                  'desktopSha256': subprocess.check_output(['sha256sum', 'target/debug/rustrss-desktop'], text=True).split()[0],
                  'storedBefore': before, 'storedAfter': after, 'refreshStatuses':refresh_statuses,'keyEvents': events, 'titlebarPointer':{'root':[rx.value,ry.value],'target':[px,py],'clicks':titlebar_clicks,'windowManager':'none; activation only, no minimize effect asserted'}, 'checks': checks, 'passed': True}
        (out / 'review-desktop-results.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
        print(json.dumps({'checks': len(checks), 'display': display}), flush=True)
    finally:
        if app and app.poll() is None:
            app.terminate(); app.wait(timeout=10)
        xvfb.terminate(); xvfb.wait(timeout=10)

if __name__ == '__main__':
    asyncio.run(main())
