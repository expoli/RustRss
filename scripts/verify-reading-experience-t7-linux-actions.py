"""T7 native Linux action probe. Run via dbus-run-session with a private HOME.

Example:
  ROOT=$(mktemp -d /tmp/rustrss-t7-linux-XXXXXX)
  mkdir -p "$ROOT/home" "$ROOT/data" "$ROOT/config" "$ROOT/cache" "$ROOT/runtime"
  chmod 700 "$ROOT/runtime"
  env HOME="$ROOT/home" XDG_DATA_HOME="$ROOT/data" XDG_CONFIG_HOME="$ROOT/config" \
    XDG_CACHE_HOME="$ROOT/cache" XDG_RUNTIME_DIR="$ROOT/runtime" \
    dbus-run-session -- python3 scripts/verify-reading-experience-t7-linux-actions.py \
    .chorus/specs/rss-reader/2026-09-30-reading-experience/evidence/t7-final/linux-actions

The launcher captures argv in place of xdg-open. It never starts a user browser
or file manager. The test bus and HOME never resolve to the user's wallet.
"""
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
import sqlite3
import subprocess
import sys

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native_probe)


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


class Pointer:
    def __init__(self, display):
        self.x11 = ctypes.CDLL('libX11.so.6')
        self.xtst = ctypes.CDLL('libXtst.so.6')
        self.x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self.x11.XOpenDisplay.restype = ctypes.c_void_p
        self.x11.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
        self.x11.XRaiseWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        self.xtst.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_ulong]
        self.xtst.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong]
        self.display = self.x11.XOpenDisplay(display.encode())
        assert self.display

    def move(self, x, y):
        assert self.xtst.XTestFakeMotionEvent(self.display, -1, round(x), round(y), 0)
        self.x11.XSync(self.display, 0)

    def button(self, down):
        assert self.xtst.XTestFakeButtonEvent(self.display, 1, int(down), 0)
        self.x11.XSync(self.display, 0)

    def raise_window(self, window):
        self.x11.XRaiseWindow(self.display, int(window, 16))
        self.x11.XSync(self.display, 0)


async def main():
    out = Path(sys.argv[1]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    root = Path(os.environ['HOME']).parent
    assert str(root).startswith('/tmp/rustrss-t7-linux-')
    assert os.environ['XDG_DATA_HOME'] == str(root / 'data')
    assert os.environ['XDG_CONFIG_HOME'] == str(root / 'config')
    assert os.environ['XDG_RUNTIME_DIR'] == str(root / 'runtime')
    assert os.environ.get('DBUS_SESSION_BUS_ADDRESS') and 'RUSTSS_AI_KEY' not in os.environ
    assert not os.environ.get('DISPLAY') and not os.environ.get('WAYLAND_DISPLAY')
    binary = Path(os.environ.get('T7_BINARY', '/tmp/rustrss-t7-linux-runtime/rustrss-desktop'))
    fixture = out / 'fixture.sqlite'
    assert fixture.is_file() and binary.is_file()
    db = root / 'fixture.sqlite'
    shutil.copy2(fixture, db)
    with sqlite3.connect(db) as conn:
        conn.execute("INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES('mcp.port',?,1790738844)", (str(free_port()),))
    bin_dir = root / 'bin'
    bin_dir.mkdir()
    opener_log = out / 'xdg-open.jsonl'
    opener_log.unlink(missing_ok=True)
    opener = bin_dir / 'xdg-open'
    opener.write_text('#!/usr/bin/python3\nimport json,os,sys\nwith open(os.environ["T7_OPENER_LOG"],"a") as f:f.write(json.dumps({"argv":sys.argv[1:],"display":os.environ.get("DISPLAY"),"home":os.environ.get("HOME")})+"\\n")\n')
    opener.chmod(0o755)
    env = dict(os.environ, PATH=str(bin_dir) + ':' + os.environ['PATH'], T7_OPENER_LOG=str(opener_log),
               RUSTSS_DB=str(db), GDK_BACKEND='x11', GDK_GL='disable', QT_QPA_PLATFORM='xcb',
               GTK_USE_PORTAL='0', RUSTSS_LOG_STDOUT='1',
               WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{free_port()}')
    display = next(':' + str(i) for i in range(100, 120) if not Path(f'/tmp/.X11-unix/X{i}').exists())
    env['DISPLAY'] = display
    xvfb_path = os.environ.get('T7_XVFB', '/tmp/rustrss-t4-xvfb/root/usr/bin/Xvfb')
    report = {'source_head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(),
              'binary_source_head': os.environ.get('T7_BINARY_HEAD', '1b4c4ca'),
              'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'fixture_sha256': hashlib.sha256(fixture.read_bytes()).hexdigest(),
              'display': display, 'private_root': str(root), 'checks': [], 'blocked': [],
              'limits': ['Xvfb has no window manager; plasmashell provides a private StatusNotifier host, but desktop panel gesture/visual appearance is outside this probe.',
                         'xdg-open is an isolated capture executable: launch argv and directory existence are verified, not a graphical browser/file manager.']}
    def record(name, status, data):
        report['checks'].append({'id': name, 'status': status, 'data': data})
        print(name, status, json.dumps(data, ensure_ascii=False), flush=True)
    xvfb = subprocess.Popen([xvfb_path, display, '-screen', '0', '1600x1000x24', '-nolisten', 'tcp', '-ac'],
                            env=env, stdout=(out / 'xvfb.log').open('w'), stderr=subprocess.STDOUT)
    service = None
    shell = None
    app = None
    app2 = None
    try:
        for _ in range(100):
            assert xvfb.poll() is None
            if Path('/tmp/.X11-unix/X' + display[1:]).exists(): break
            await asyncio.sleep(.1)
        assert Path('/tmp/.X11-unix/X' + display[1:]).exists()
        subprocess.run(['dbus-update-activation-environment', 'DISPLAY', 'QT_QPA_PLATFORM'], env=env, check=True,
                       stdout=(out/'dbus-activation.log').open('w'), stderr=subprocess.STDOUT)
        shell = subprocess.Popen(['/usr/bin/plasmashell'], env=env,
                                 stdout=(out/'plasmashell.log').open('w'), stderr=subprocess.STDOUT)
        for _ in range(100):
            owner = subprocess.run(['gdbus','call','--session','--dest','org.freedesktop.DBus',
                                    '--object-path','/org/freedesktop/DBus','--method','org.freedesktop.DBus.NameHasOwner',
                                    'org.kde.StatusNotifierWatcher'],env=env,capture_output=True,text=True,timeout=10)
            if '(true,)' in owner.stdout: break
            await asyncio.sleep(.1)
        report['status_notifier_watcher_at_launch'] = '(true,)' in owner.stdout
        service = subprocess.Popen(['/usr/bin/ksecretd'], env=env, stdout=(out / 'ksecretd.log').open('w'), stderr=subprocess.STDOUT)
        with (out / 'app.log').open('w') as log:
            app = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log)
        probe = native_probe.Probe({'root': str(root), 'inspector_port': int(env['WEBKIT_INSPECTOR_HTTP_SERVER'].rsplit(':', 1)[1])})
        for _ in range(200):
            try:
                await probe.js('1')
                break
            except (OSError, IndexError):
                assert app.poll() is None, 'app exited while opening inspector'
                await asyncio.sleep(.1)
        await probe.until('!!document.querySelector("#tags li[data-tag-id]")')
        for _ in range(100):
            windows = subprocess.check_output(['xwininfo', '-root', '-tree'], env=env, text=True)
            if re.search(r'(0x[0-9a-f]+) "RustRss"', windows): break
            await asyncio.sleep(.1)
        (out / 'windows.txt').write_text(windows)
        match = re.search(r'(0x[0-9a-f]+) "RustRss"', windows)
        assert match, 'owned RustRss X11 window absent'
        window = match.group(1)
        pointer = Pointer(display)
        pointer.raise_window(window)

        async def read(expression):
            return json.loads(await probe.js('JSON.stringify(' + expression + ')'))

        async def invoke(name, args=None):
            await probe.js('window.__T7_CALL="pending";window.__TAURI__.core.invoke(' + json.dumps(name) + ',' + json.dumps(args or {}) + ').then(v=>window.__T7_CALL={ok:true,value:v},e=>window.__T7_CALL={ok:false,error:String(e)});true')
            await probe.until('window.__T7_CALL!=="pending"')
            return await read('window.__T7_CALL')

        def db_tags():
            with sqlite3.connect(db) as conn:
                return conn.execute('SELECT id,name,pinned,sort_order FROM tags ORDER BY pinned DESC,sort_order,id').fetchall()

        # A real XTest pointer gesture into WebKitGTK, rather than synthetic DOM DnD.
        await probe.js('window.__T7_DRAG=[];for(const name of ["dragstart","dragover","drop","dragend"])document.querySelector("#tags").addEventListener(name,e=>{const row=e.target.closest("li");setTimeout(()=>window.__T7_DRAG.push({name,trusted:e.isTrusted,target:row?.dataset.tagId,prevented:e.defaultPrevented,dropEffect:e.dataTransfer?.dropEffect,classes:row?.className}),0)},false);true')
        points = await read('(() => {const rows=[...document.querySelectorAll("#tags li[data-tag-id]")].slice(0,3);return rows.map(n=>{const r=n.getBoundingClientRect();return {id:n.dataset.tagId,x:r.left+Math.min(40,r.width/2),y:r.top+r.height/2,h:r.height}})})()')
        before = db_tags()
        source, target = points[0], points[1]
        pointer.move(source['x'], source['y']); await asyncio.sleep(.2); pointer.button(True)
        for i in range(1, 26):
            x = source['x'] + (target['x'] - source['x']) * i / 25
            y = source['y'] + (target['y'] + target['h'] / 4 - source['y']) * i / 25
            pointer.move(x, y)
            await asyncio.sleep(.035)
        await asyncio.sleep(.35)
        pointer.button(False)
        await asyncio.sleep(.8)
        drag_events = await read('window.__T7_DRAG')
        after = db_tags()
        record('M08.sort.tag-native-drag', 'pass' if after != before and any(e['name']=='drop' and e['trusted'] for e in drag_events) else 'blocked',
               {'before': before, 'after': after, 'events': drag_events})
        subprocess.run(['import', '-display', display, '-window', window, str(out / 'tag-drag.png')], env=env, check=True)
        # Folder rows are not draggable: only feed rows use native DnD, as the UI source states.
        feed_drag = await read('(() => ({folder:[...document.querySelectorAll("#feeds li[data-folder-id]")].map(n=>({id:n.dataset.folderId,draggable:n.draggable})).slice(0,2),feed:[...document.querySelectorAll("#feeds li[data-feed-id]")].map(n=>({id:n.dataset.feedId,draggable:n.draggable})).slice(0,2)}))()')
        record('M08.sort.folder-applicability', 'pass', feed_drag)
        await probe.js('window.__T7_FEED_DRAG=[];for(const name of ["dragstart","dragover","drop","dragend"])document.querySelector("#feeds").addEventListener(name,e=>{const row=e.target.closest("li");setTimeout(()=>window.__T7_FEED_DRAG.push({name,trusted:e.isTrusted,target:row?.dataset.feedId,prevented:e.defaultPrevented,dropEffect:e.dataTransfer?.dropEffect,classes:row?.className}),0)},false);true')
        await probe.js('document.querySelector("#feeds li[data-feed-id]").scrollIntoView({block:"center"});true')
        feed_points = await read('(() => [...document.querySelectorAll("#feeds li[data-feed-id]")].slice(0,2).map(n=>{const r=n.getBoundingClientRect();return {id:n.dataset.feedId,x:r.left+Math.min(40,r.width/2),y:r.top+r.height/2,h:r.height}}))()')
        assert all(0<p['y']<820 for p in feed_points),feed_points
        with sqlite3.connect(db) as conn:
            feed_before = conn.execute('SELECT id,folder_id,position FROM feeds ORDER BY id').fetchall()
        source, target = feed_points
        pointer.raise_window(window)
        pointer.move(source['x'],source['y']); await asyncio.sleep(.2); pointer.button(True)
        for i in range(1,26):
            pointer.move(source['x']+(target['x']-source['x'])*i/25,
                         source['y']+(target['y']-target['h']/4-source['y'])*i/25)
            await asyncio.sleep(.035)
        await asyncio.sleep(.35); pointer.button(False); await asyncio.sleep(.7)
        with sqlite3.connect(db) as conn:
            feed_after = conn.execute('SELECT id,folder_id,position FROM feeds ORDER BY id').fetchall()
        feed_events = await read('window.__T7_FEED_DRAG')
        record('M08.sort.feed-native-drag', 'pass' if feed_after != feed_before and any(e['name']=='drop' and e['trusted'] for e in feed_events) else 'blocked',
               {'before':feed_before,'after':feed_after,'points':feed_points,'events':feed_events})
        subprocess.run(['import','-display',display,'-window',window,str(out/'feed-drag.png')],env=env,check=True)

        # Private Secret Service; service registration and UI key read/write outcomes.
        names = subprocess.run(['gdbus', 'call', '--session', '--dest', 'org.freedesktop.DBus', '--object-path', '/org/freedesktop/DBus', '--method', 'org.freedesktop.DBus.ListNames'],
                               env=env, capture_output=True, text=True, timeout=10)
        (out / 'private-bus-names.txt').write_text(names.stdout + names.stderr)
        tray_names = [name for name in re.findall(r"'([^']+)'", names.stdout) if 'StatusNotifier' in name or 'indicator' in name.lower()]
        tray_items = subprocess.run(['gdbus','call','--session','--dest','org.kde.StatusNotifierWatcher',
                                     '--object-path','/StatusNotifierWatcher','--method','org.freedesktop.DBus.Properties.Get',
                                     'org.kde.StatusNotifierWatcher','RegisteredStatusNotifierItems'],
                                    env=env,capture_output=True,text=True,timeout=10)
        (out/'tray-items.txt').write_text(tray_items.stdout+tray_items.stderr)
        record('M18.tray-registration', 'pass' if tray_items.returncode==0 and 'NotificationItem' in tray_items.stdout else 'blocked',
               {'names':tray_names, 'plasmashell_alive':shell.poll() is None,
                'watcher_at_launch':report['status_notifier_watcher_at_launch'],
                'registered_items':tray_items.stdout, 'error':tray_items.stderr})
        initial = await invoke('get_ai_settings')
        key = 't7-private-test-key'
        save = await invoke('save_ai_settings', {'provider':'openai','model':'gpt-test','baseUrl':'http://127.0.0.1:1/v1', 'translateTarget':'zh-CN','apiKey':key,'maxOutputTokens':128})
        status = await invoke('get_ai_settings')
        clear = await invoke('save_ai_settings', {'provider':'openai','model':'gpt-test','baseUrl':'http://127.0.0.1:1/v1', 'translateTarget':'zh-CN','apiKey':'','maxOutputTokens':128})
        cleared = await invoke('get_ai_settings')
        redacted = lambda value: json.loads(json.dumps(value).replace(key, '[REDACTED]'))
        key_result = {'private_bus': names.returncode == 0 and 'org.freedesktop.secrets' in names.stdout,
                      'initial': redacted(initial), 'save': redacted(save), 'status': redacted(status),
                      'clear': redacted(clear), 'cleared': redacted(cleared)}
        record('M16.key-status-save-clear', 'pass' if save['ok'] and status['value']['has_key'] and clear['ok'] and not cleared['value']['has_key'] else 'blocked', key_result)
        with sqlite3.connect(db) as conn:
            hits = conn.execute("SELECT count(*) FROM settings WHERE instr(value,?)>0", (key,)).fetchone()[0]
        record('M16.key-not-in-sqlite', 'pass' if hits == 0 else 'fail', {'matching_setting_rows': hits})

        # Local refusal is deterministic and never contacts an external provider.
        await invoke('set_log_level', {'level':'info'})
        before_log = ''.join(p.read_text(errors='replace') for p in (root/'data'/'rustrss'/'logs').glob('*.log'))
        scope_info = await invoke('list_scope_total', {'kind':'feed','id':1})
        test_info = await invoke('test_ai_connection')
        await asyncio.sleep(.2)
        mid_log = ''.join(p.read_text(errors='replace') for p in (root/'data'/'rustrss'/'logs').glob('*.log'))
        await invoke('set_log_level', {'level':'debug'})
        scope_debug = await invoke('list_scope_total', {'kind':'feed','id':1})
        test_debug = await invoke('test_ai_connection')
        await asyncio.sleep(.2)
        after_log = ''.join(p.read_text(errors='replace') for p in (root/'data'/'rustrss'/'logs').glob('*.log'))
        info_delta = mid_log[len(before_log):]
        debug_delta = after_log[len(mid_log):]
        record('M18.log-level-lines', 'pass' if 'list_scope_total feed#1:' not in info_delta and 'list_scope_total feed#1:' in debug_delta else 'blocked',
               {'info_scope':scope_info, 'debug_scope':scope_debug,
                'info_debug_lines': [x for x in info_delta.splitlines() if 'list_scope_total feed#1:' in x],
                'debug_debug_lines': [x for x in debug_delta.splitlines() if 'list_scope_total feed#1:' in x]})
        record('M16.test-failure', 'pass' if not test_info['ok'] and not test_debug['ok'] else 'fail',
               {'info_result':redacted(test_info),'debug_result':redacted(test_debug)})
        with sqlite3.connect(db) as conn:
            level = conn.execute("SELECT value FROM settings WHERE key='log.level'").fetchone()
        record('M18.log-level-store', 'pass' if level == ('debug',) else 'fail', {'row':level})

        # Exercise a reader URL and the About source control through their UI.
        entry_id = await read('Number(document.querySelector("#entries li[data-id]").dataset.id)')
        with sqlite3.connect(db) as conn:
            article_url = conn.execute('SELECT url FROM entries WHERE id=?',(entry_id,)).fetchone()[0]
        assert article_url and article_url.startswith(('http://','https://'))
        await probe.js('document.querySelector("#entries li[data-id]").click();true')
        await probe.until('!!document.querySelector("#act-more")')
        await probe.js('document.querySelector("#act-more").click();true')
        await probe.until('!!document.querySelector("#ctx-menu")')
        await probe.js('[...document.querySelectorAll("#ctx-menu button")].find(n=>n.textContent.includes("Open in browser")).click();true')
        await asyncio.sleep(.3)
        # Exercise the About source control and log-folder control in native WebKitGTK.
        await probe.js('document.querySelector("#btn-settings").click();document.querySelector("#tab-general").click();true')
        await probe.until('!document.querySelector("#pane-general").classList.contains("hidden")')
        await probe.js('document.querySelector("#about-open-source").click();true')
        await asyncio.sleep(.3)
        logs_call = await invoke('open_logs_dir')
        await asyncio.sleep(.3)
        invalid = await invoke('open_external', {'url':'file:///tmp/forbidden'})
        opener_rows = [json.loads(x) for x in opener_log.read_text().splitlines()] if opener_log.exists() else []
        logs_dir = root/'data'/'rustrss'/'logs'
        record('M18.external-and-log-folder', 'pass' if len(opener_rows) == 3 and opener_rows[0]['argv']==[article_url] and opener_rows[1]['argv']==['https://github.com/expoli/RustRss'] and opener_rows[2]['argv']==[str(logs_dir)] and logs_dir.is_dir() and logs_call['ok'] and not invalid['ok'] else 'fail',
               {'entry_id':entry_id,'launches':opener_rows, 'logs_dir_exists':logs_dir.is_dir(), 'logs_call':logs_call, 'invalid_scheme':invalid})
        subprocess.run(['import','-display',display,'-window',window,str(out/'general-actions.png')],env=env,check=True)

        # Observe the real StatusNotifier item's D-Bus menu and try its toggle
        # event after the Rust close command hides the owned X11 window.
        item = re.search(r"(:\d+\.\d+)(/org/ayatana/NotificationItem/[^'\]]+)",tray_items.stdout)
        if item:
            item_name, item_path = item.groups()
            menu_prop = subprocess.run(['gdbus','call','--session','--dest',item_name,'--object-path',item_path,
                '--method','org.freedesktop.DBus.Properties.Get','org.kde.StatusNotifierItem','Menu'],
                env=env,capture_output=True,text=True,timeout=10)
            (out/'tray-menu-property.txt').write_text(menu_prop.stdout+menu_prop.stderr)
            menu_match = re.search(r"'(/[^']+)'",menu_prop.stdout)
            if menu_match:
                menu_path = menu_match.group(1)
                layout = subprocess.run(['gdbus','call','--session','--dest',item_name,'--object-path',menu_path,
                    '--method','com.canonical.dbusmenu.GetLayout','--','0','-1','[]'],
                    env=env,capture_output=True,text=True,timeout=10)
                (out/'tray-menu-layout.txt').write_text(layout.stdout+layout.stderr)
                toggle_match = re.search(r"\((\d+), \{[^}]*Show/Hide Window",layout.stdout)
                if toggle_match:
                    tray_setting = await invoke('set_ui_close_action',{'action':'tray'})
                    tray_close = await invoke('window_close')
                    await asyncio.sleep(.3)
                    hidden = subprocess.run(['xwininfo','-id',window],env=env,capture_output=True,text=True,timeout=10)
                    toggle = subprocess.run(['busctl','--user','call',item_name,menu_path,'com.canonical.dbusmenu',
                        'Event','isvu',toggle_match.group(1),'clicked','i','0','0'],
                        env=env,capture_output=True,text=True,timeout=10)
                    await asyncio.sleep(.4)
                    restored = subprocess.run(['xwininfo','-id',window],env=env,capture_output=True,text=True,timeout=10)
                    tray_data = {'item':item_name+item_path,'menu':menu_path,'layout':layout.stdout,
                                 'setting':tray_setting.get('value',{}).get('close_action'),'close':tray_close,
                                 'process_after_hide':app.poll(),'hidden':re.search(r'Map State: (\S+)',hidden.stdout).group(1) if 'Map State:' in hidden.stdout else hidden.stderr,
                                 'toggle_returncode':toggle.returncode,'toggle_stdout':toggle.stdout,'toggle_stderr':toggle.stderr,
                                 'restored':re.search(r'Map State: (\S+)',restored.stdout).group(1) if 'Map State:' in restored.stdout else restored.stderr}
                    record('M18.tray-hide-restore','pass' if tray_close['ok'] and tray_data['hidden']=='IsUnMapped' and toggle.returncode==0 and tray_data['restored']=='IsViewable' and app.poll() is None else 'blocked',tray_data)
                else:
                    record('M18.tray-hide-restore','blocked',{'menu_layout':layout.stdout,'menu_error':layout.stderr})
            else:
                record('M18.tray-hide-restore','blocked',{'menu_property':menu_prop.stdout,'menu_error':menu_prop.stderr})
        else:
            record('M18.tray-hide-restore','blocked',{'registered_items':tray_items.stdout})

        # Close in exit mode: process status is the system effect.
        close_setting = await invoke('set_ui_close_action', {'action':'exit'})
        try:
            await probe.js('document.querySelector("#btn-win-close").click();true')
        except Exception as error:
            # The inspector WebSocket may close before it can reply: that is
            # expected when the UI really exits. The process code decides.
            report['close_inspector_error'] = repr(error)
        for _ in range(100):
            if app.poll() is not None: break
            await asyncio.sleep(.1)
        record('M18.exit', 'pass' if app.poll() == 0 else 'fail', {'setting':close_setting.get('value',{}).get('close_action'), 'exit_code':app.poll()})

        # A second, still private instance uses a throwaway environment key so
        # the AI request can reach the local endpoint failure path despite the
        # unavailable Secret Service collection in the first instance.
        env2 = dict(env, RUSTSS_AI_KEY='t7-local-env-only-key',
                    WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{free_port()}')
        with (out/'ai-local-failure-app.log').open('w') as log:
            app2 = subprocess.Popen([str(binary)],env=env2,stdout=log,stderr=log)
        probe2 = native_probe.Probe({'root':str(root),'inspector_port':int(env2['WEBKIT_INSPECTOR_HTTP_SERVER'].rsplit(':',1)[1])})
        for _ in range(200):
            try:
                await probe2.js('1');break
            except (OSError,IndexError):
                assert app2.poll() is None
                await asyncio.sleep(.1)
        await probe2.until('!!window.__TAURI__?.core')
        await probe2.js('window.__T7_AI="pending";window.__TAURI__.core.invoke("get_ai_settings").then(v=>window.__T7_AI={ok:true,value:v},e=>window.__T7_AI={ok:false,error:String(e)});true')
        await probe2.until('window.__T7_AI!=="pending"')
        env_ai = json.loads(await probe2.js('JSON.stringify(window.__T7_AI)'))
        await probe2.js('window.__T7_AI="pending";window.__TAURI__.core.invoke("test_ai_connection").then(v=>window.__T7_AI={ok:true,value:v},e=>window.__T7_AI={ok:false,error:String(e)});true')
        await probe2.until('window.__T7_AI!=="pending"')
        local_failure = json.loads(await probe2.js('JSON.stringify(window.__T7_AI)'))
        record('M16.test-local-failure', 'pass' if env_ai['ok'] and env_ai['value']['key_source']['kind']=='env' and not local_failure['ok'] else 'fail',
               {'key_source':env_ai.get('value',{}).get('key_source'), 'result':local_failure,
                'endpoint':'http://127.0.0.1:1/v1'})
    except Exception as error:
        report['error'] = repr(error)
        raise
    finally:
        if app and app.poll() is None:
            app.terminate(); app.wait(timeout=10)
        if app2 and app2.poll() is None:
            app2.terminate(); app2.wait(timeout=10)
        if service and service.poll() is None:
            service.terminate(); service.wait(timeout=10)
        if shell and shell.poll() is None:
            shell.terminate(); shell.wait(timeout=10)
        if xvfb.poll() is None:
            xvfb.terminate(); xvfb.wait(timeout=10)
        report['blocked'] = [c['id'] for c in report['checks'] if c['status']=='blocked']
        report['passed'] = all(c['status']=='pass' for c in report['checks']) and 'error' not in report
        (out/'results.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')


if __name__ == '__main__':
    asyncio.run(main())
