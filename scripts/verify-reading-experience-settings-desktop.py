"""Settings checks in a rebuilt Linux WebKitGTK app with a synthetic DB and Xvfb."""
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
import tempfile
import urllib.error
import urllib.request

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native_probe)

def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]

async def main():
    out = Path(sys.argv[1]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix='rustrss-t5-desktop-'))
    (root / 'home').mkdir()
    (root / 'runtime').mkdir(mode=0o700)
    db = root / 'fixture.sqlite'
    shutil.copy2('/tmp/rustrss-t5-fixture.sqlite', db)
    with sqlite3.connect(db) as conn:
        conn.execute("INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES('mcp.port',?,1790738844)", (str(free_port()),))
        conn.execute("INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES('ui.locale','en',1790738844)")
    env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
               XDG_RUNTIME_DIR=str(root / 'runtime'), RUSTSS_DB=str(db), GDK_GL='disable',
               WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{free_port()}')
    env.pop('DISPLAY', None)
    env.pop('WAYLAND_DISPLAY', None)
    display = next(':' + str(i) for i in range(70, 90) if not Path(f'/tmp/.X11-unix/X{i}').exists())
    xvfb_binary = os.environ.get('T5_XVFB') or shutil.which('Xvfb') or '/tmp/rustrss-t4-xvfb/root/usr/bin/Xvfb'
    with (out / 'desktop-xvfb.log').open('w') as log:
        xvfb = subprocess.Popen([xvfb_binary, display, '-screen', '0', '1600x1000x24', '-nolisten', 'tcp', '-ac'], env=env, stdout=log, stderr=subprocess.STDOUT)
    app = None
    results = {'checks': [], 'limits': ['Xvfb lacks a window manager; title-bar and tray gestures were not exercised.']}
    def check(name, value, detail=None):
        assert value, f'{name}: {detail}'
        results['checks'].append({'name': name, 'detail': detail})
    try:
        for _ in range(200):
            assert xvfb.poll() is None
            if Path('/tmp/.X11-unix/X' + display[1:]).exists(): break
            await asyncio.sleep(.1)
        env.update(DISPLAY=display, GDK_BACKEND='x11')
        with (out / 'desktop-app.log').open('w') as log:
            app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log)
        probe = native_probe.Probe({'root': str(root), 'inspector_port': int(env['WEBKIT_INSPECTOR_HTTP_SERVER'].split(':')[-1])})
        for _ in range(200):
            try:
                await probe.js('1')
                break
            except (OSError, IndexError):
                assert app.poll() is None
                await asyncio.sleep(.1)
        await probe.until('!!document.querySelector("#entries li[data-id]")')
        for _ in range(100):
            listing = subprocess.check_output(['xwininfo', '-root', '-tree'], env=env, text=True)
            if re.search(r'(0x[0-9a-f]+) "RustRss"', listing): break
            await asyncio.sleep(.1)
        (out / 'desktop-windows.txt').write_text(listing)
        found = re.search(r'(0x[0-9a-f]+) "RustRss"', listing) or re.search(r'(0x[0-9a-f]+) "rustrss-desktop": \("rustrss-desktop"', listing)
        assert found, 'App X11 window absent'
        window = int(found.group(1), 16)
        x11 = ctypes.CDLL('libX11.so.6')
        x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        x11.XOpenDisplay.restype = ctypes.c_void_p
        x11.XResizeWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_uint, ctypes.c_uint]
        x11.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
        handle = x11.XOpenDisplay(display.encode())
        assert handle
        x11.XResizeWindow(handle, window, 920, 850)
        x11.XSync(handle, 0)
        await asyncio.sleep(.5)
        width = await probe.js('innerWidth')
        check('narrow-desktop-viewport', width <= 960, width)
        await probe.js('document.querySelector("#btn-settings").click();true')
        await probe.until('!document.querySelector("#settings-overlay").classList.contains("hidden")')
        categories = ['appearance', 'reading', 'subscriptions', 'ai', 'mcp', 'data', 'general']
        for name in categories:
            row = await probe.js(f'''(() => {{const t=document.querySelector('#tab-{name}');return {{visible:t.getClientRects().length>0,summary:document.querySelector('#settings-summary-{name}')?.textContent}}}})()''')
            check('desktop-category-' + name, row['visible'] and bool(row['summary']), row)
            await probe.js(f'document.querySelector("#tab-{name}").click();true')
            check('desktop-pane-' + name, await probe.js(f'!document.querySelector("#pane-{name}").classList.contains("hidden")'))
        await probe.js('document.querySelector("#tab-appearance").click();document.querySelector("#appearance-editor .theme-advanced").open=true;true')
        fields = await probe.js('''(() => {const q=s=>!!document.querySelector(s)?.getClientRects().length;return {sidebar:q('#appearance-editor [data-theme-field="chrome.sidebar_width"]'),list:q('#appearance-editor [data-theme-field="chrome.list_width"]')}})()''')
        check('narrow-desktop-keeps-column-widths', all(fields.values()), fields)
        await probe.js('document.querySelector("#tab-reading").click();document.querySelector("#reading-editor .theme-advanced").open=true;true')
        fields = await probe.js('''(() => {const q=s=>!!document.querySelector(s)?.getClientRects().length;return {readerWidth:q('#reading-editor [data-theme-field="reader.width"]'),layout:q('#reading-editor [data-theme-field="reader.layout"]'),markRead:q('#set-mark-read')}})()''')
        check('narrow-desktop-keeps-reading-layout', all(fields.values()), fields)
        await probe.js('document.querySelector("#tab-data").click();true')
        data = await probe.js('''(() => {const q=s=>!!document.querySelector(s)?.getClientRects().length;return {import:q('#act-import-opml'),export:q('#act-export-opml'),backup:q('#act-backup-db'),restore:q('#act-restore-db'),androidHint:q('#pane-data .m-only')}})()''')
        check('desktop-data-actions', data['import'] and data['export'] and data['backup'] and data['restore'] and not data['androidHint'], data)
        await probe.js('document.querySelector("#tab-general").click();true')
        general = await probe.js('''(() => {const q=s=>!!document.querySelector(s)?.getClientRects().length;return {locale:q('#set-language'),close:q('#set-close-action'),logs:q('#act-open-logs'),level:q('#set-log-level')}})()''')
        check('desktop-general-actions', all(general.values()), general)
        await probe.js('document.querySelector("#tab-mcp").click();true')
        mcp = await probe.js('''(() => {const q=s=>!!document.querySelector(s)?.getClientRects().length;return {enabled:q('#set-mcp-enabled'),port:q('#mcp-port'),status:q('#mcp-status'),snippet:q('#mcp-copy-snippet'),rotate:q('#mcp-rotate'),write:q('#set-mcp-write-enabled'),danger:q('#set-mcp-dangerous-enabled'),generate:q('#mcp-write-generate')}})()''')
        check('desktop-mcp-actions', all(mcp.values()), mcp)
        with sqlite3.connect(db) as conn:
            original_port = conn.execute("SELECT value FROM settings WHERE key='mcp.port'").fetchone()[0]
        await probe.js('window.__T5_PORT_RESULT="pending";window.__TAURI__.core.invoke("set_mcp_port",{port:1}).then(()=>window.__T5_PORT_RESULT="accepted",e=>window.__T5_PORT_RESULT=String(e));true')
        await probe.until('window.__T5_PORT_RESULT!=="pending"')
        direct_result = await probe.js('window.__T5_PORT_RESULT')
        with sqlite3.connect(db) as conn:
            after_direct = conn.execute("SELECT value FROM settings WHERE key='mcp.port'").fetchone()[0]
        check('mcp-core-rejects-invalid-without-write', direct_result != 'accepted' and after_direct == original_port, {'result': direct_result, 'port': after_direct})
        await probe.js('document.querySelector("#mcp-port").value="1";document.querySelector("#mcp-port").dispatchEvent(new Event("change",{bubbles:true}));true')
        await probe.until('document.querySelector("#mcp-port").getAttribute("aria-invalid")==="true"')
        check('mcp-port-error-linked', await probe.js('document.querySelector("#mcp-port").getAttribute("aria-invalid")==="true"'))
        with sqlite3.connect(db) as conn:
            db_port = conn.execute("SELECT value FROM settings WHERE key='mcp.port'").fetchone()[0]
        check('mcp-invalid-port-no-write', db_port != '1', db_port)
        await probe.js('document.querySelector("#mcp-port").value="' + db_port + '";document.querySelector("#mcp-port").dispatchEvent(new Event("change",{bubbles:true}));true')
        await probe.until('document.querySelector("#mcp-port").getAttribute("aria-invalid")==="false"')
        async def mcp_state():
            await probe.js('window.__T5_MCP_STATE="pending";window.__TAURI__.core.invoke("get_mcp_settings").then(v=>window.__T5_MCP_STATE=v,e=>window.__T5_MCP_STATE={error:String(e)});true')
            await probe.until('window.__T5_MCP_STATE!=="pending"')
            state = await probe.js('window.__T5_MCP_STATE')
            assert 'error' not in state, state
            return state
        def init_status(token):
            request = urllib.request.Request(f'http://127.0.0.1:{db_port}/mcp',
                data=json.dumps({'jsonrpc':'2.0','id':1,'method':'initialize','params':{
                    'protocolVersion':'2026-07-28','capabilities':{},'clientInfo':{'name':'t5-probe','version':'0'}}}).encode(),
                headers={'Content-Type':'application/json','Accept':'application/json, text/event-stream',
                         'Authorization':'Bearer ' + token}, method='POST')
            try:
                with urllib.request.urlopen(request, timeout=5) as response: return response.status
            except urllib.error.HTTPError as error: return error.code
        initial = await mcp_state()
        check('mcp-default-write-disabled', not initial['write_enabled'] and not initial['dangerous_enabled'] and not initial['write_token'])
        await probe.js('document.querySelector("#set-mcp-enabled").checked=true;document.querySelector("#set-mcp-enabled").dispatchEvent(new Event("change",{bubbles:true}));true')
        await probe.until('document.querySelector("#mcp-status").textContent.includes("Running")')
        live = await mcp_state()
        check('mcp-live-loopback', live['enabled'] and live['running'] and live['loopback_only'] and live['url'] == f'http://127.0.0.1:{db_port}/mcp', {'url':live['url']})
        with urllib.request.urlopen(f'http://127.0.0.1:{db_port}/health', timeout=5) as response:
            health = response.read().decode()
        check('mcp-health', health == 'ok')
        check('mcp-read-token-auth', init_status(live['token']) == 200 and init_status('t5-invalid-token') == 401)
        await probe.js('document.querySelector("#mcp-copy-snippet").click();true')
        await asyncio.sleep(.2)
        copied_snippet = subprocess.check_output(['xsel','-ob'],env=env,text=True)
        check('mcp-snippet-clipboard', live['token'] in copied_snippet and str(db_port) in copied_snippet)
        await probe.js('document.querySelector("#mcp-rotate").click();true')
        await probe.until('document.querySelector("#mcp-status").textContent.includes("token:")')
        for _ in range(30):
            rotated = await mcp_state()
            if rotated['token'] != live['token']: break
            await asyncio.sleep(.1)
        check('mcp-read-token-rotation', rotated['token'] != live['token'] and init_status(live['token']) == 401 and init_status(rotated['token']) == 200)
        await probe.js('document.querySelector("#mcp-write-generate").click();true')
        for _ in range(30):
            generated = await mcp_state()
            if generated['write_token']: break
            await asyncio.sleep(.1)
        check('mcp-write-token-generate', bool(generated['write_token']) and not generated['write_enabled'])
        await probe.js('document.querySelector("#mcp-write-copy").click();true')
        await asyncio.sleep(.2)
        check('mcp-write-token-clipboard', subprocess.check_output(['xsel','-ob'],env=env,text=True) == generated['write_token'])
        await probe.js('document.querySelector("#set-mcp-write-enabled").checked=true;document.querySelector("#set-mcp-write-enabled").dispatchEvent(new Event("change",{bubbles:true}));true')
        for _ in range(30):
            write_on = await mcp_state()
            if write_on['write_enabled']: break
            await asyncio.sleep(.1)
        check('mcp-write-toggle', write_on['write_enabled'])
        await probe.js('document.querySelector("#set-mcp-dangerous-enabled").checked=true;document.querySelector("#set-mcp-dangerous-enabled").dispatchEvent(new Event("change",{bubbles:true}));true')
        await probe.until('!document.querySelector("#generic-confirm-overlay").classList.contains("hidden")')
        await probe.js('document.querySelector("#generic-confirm-cancel").click();true')
        check('mcp-danger-cancel-no-write', not (await mcp_state())['dangerous_enabled'] and not await probe.js('document.querySelector("#set-mcp-dangerous-enabled").checked'))
        await probe.js('document.querySelector("#set-mcp-dangerous-enabled").checked=true;document.querySelector("#set-mcp-dangerous-enabled").dispatchEvent(new Event("change",{bubbles:true}));true')
        await probe.until('!document.querySelector("#generic-confirm-overlay").classList.contains("hidden")')
        await probe.js('document.querySelector("#generic-confirm-ok").click();true')
        for _ in range(30):
            danger_on = await mcp_state()
            if danger_on['dangerous_enabled']: break
            await asyncio.sleep(.1)
        check('mcp-danger-confirm-write', danger_on['dangerous_enabled'])
        await probe.js('document.querySelector("#mcp-write-rotate").click();true')
        for _ in range(30):
            write_rotated = await mcp_state()
            if write_rotated['write_token'] != generated['write_token']: break
            await asyncio.sleep(.1)
        check('mcp-write-token-rotate', bool(write_rotated['write_token']) and write_rotated['write_token'] != generated['write_token'])
        await probe.js('document.querySelector("#mcp-write-clear").click();true')
        for _ in range(30):
            cleared = await mcp_state()
            if not cleared['write_token']: break
            await asyncio.sleep(.1)
        check('mcp-write-token-clear', cleared['write_token'] is None)
        await probe.js('document.querySelector("#set-mcp-enabled").checked=false;document.querySelector("#set-mcp-enabled").dispatchEvent(new Event("change",{bubbles:true}));true')
        for _ in range(30):
            stopped = await mcp_state()
            if not stopped['running']: break
            await asyncio.sleep(.1)
        check('mcp-service-stop', not stopped['running'])
        await probe.js('window.__T5_BACKUP="pending";window.__TAURI__.core.invoke("backup_db").then(v=>window.__T5_BACKUP=v,e=>window.__T5_BACKUP={error:String(e)});true')
        await asyncio.sleep(.8)
        backup_windows = subprocess.check_output(['xwininfo','-root','-tree'],env=env,text=True)
        (out / 'desktop-backup-windows.txt').write_text(backup_windows)
        picker = re.search(r'(0x[0-9a-f]+) "Select Folder"', backup_windows)
        if picker:
            subprocess.run(['import','-display',display,'-window',picker.group(1),str(out / 'desktop-backup-picker.png')],env=env,check=True)
            backup_dir = root / 'home' / 'backup-output'
            backup_dir.mkdir()
            xtst = ctypes.CDLL('libXtst.so.6')
            xtst.XTestFakeMotionEvent.argtypes = [ctypes.c_void_p,ctypes.c_int,ctypes.c_int,ctypes.c_int,ctypes.c_ulong]
            xtst.XTestFakeButtonEvent.argtypes = [ctypes.c_void_p,ctypes.c_uint,ctypes.c_int,ctypes.c_ulong]
            def tap(x,y):
                xtst.XTestFakeMotionEvent(handle,-1,x,y,0)
                xtst.XTestFakeButtonEvent(handle,1,1,0)
                x11.XSync(handle,0)
                import time
                time.sleep(.05)
                xtst.XTestFakeButtonEvent(handle,1,0,0)
                x11.XSync(handle,0)
            tap(65,60)
            await asyncio.sleep(1)
            (out / 'desktop-backup-home-windows.txt').write_text(subprocess.check_output(['xwininfo','-root','-tree'],env=env,text=True))
            subprocess.run(['import','-display',display,'-window',picker.group(1),str(out / 'desktop-backup-home.png')],env=env,check=False)
            tap(244,84)
            await asyncio.sleep(.1)
            tap(1045,799)
            await asyncio.sleep(.5)
            backup_result = await probe.js('window.__T5_BACKUP')
            results['backupDialogResult'] = backup_result
            backup_path = Path(backup_result)
            with sqlite3.connect(backup_path) as snap:
                integrity = snap.execute('PRAGMA integrity_check').fetchone()[0]
                article_count = snap.execute('SELECT count(*) FROM entries').fetchone()[0]
            check('db-backup-native-snapshot', backup_path.parent == backup_dir and integrity == 'ok' and article_count == 30,
                  {'integrity':integrity,'articles':article_count,'bytes':backup_path.stat().st_size})
            await probe.js('window.__T5_MARK="pending";window.__TAURI__.core.invoke("set_mcp_write_enabled",{enabled:false}).then(v=>window.__T5_MARK=v,e=>window.__T5_MARK={error:String(e)});true')
            await probe.until('window.__T5_MARK!=="pending"')
            check('db-restore-marker-after-backup', not (await mcp_state())['write_enabled'])
            await probe.js('window.__T5_RESTORE="pending";window.__TAURI__.core.invoke("restore_db").then(v=>window.__T5_RESTORE=v,e=>window.__T5_RESTORE={error:String(e)});true')
            await asyncio.sleep(.8)
            restore_windows = subprocess.check_output(['xwininfo','-root','-tree'],env=env,text=True)
            (out / 'desktop-restore-windows.txt').write_text(restore_windows)
            restore_picker = re.search(r'(0x[0-9a-f]+) "[^"]*Open[^"]*"', restore_windows)
            if restore_picker:
                subprocess.run(['import','-display',display,'-window',restore_picker.group(1),str(out / 'desktop-restore-picker.png')],env=env,check=True)
                tap(230,36)
                await asyncio.sleep(.08)
                tap(230,36)
                await asyncio.sleep(.5)
                subprocess.run(['import','-display',display,'-window',restore_picker.group(1),str(out / 'desktop-restore-after-folder.png')],env=env,check=True)
                tap(1045,799)
                await asyncio.sleep(.5)
                confirm_windows = subprocess.check_output(['xwininfo','-root','-tree'],env=env,text=True)
                (out / 'desktop-restore-confirm-windows.txt').write_text(confirm_windows)
                confirm_window = re.search(r'(0x[0-9a-f]+) "Restore from backup"',confirm_windows)
                if confirm_window:
                    subprocess.run(['import','-display',display,'-window',confirm_window.group(1),str(out / 'desktop-restore-confirm.png')],env=env,check=True)
                    tap(160,215)
                    await probe.until('window.__T5_RESTORE!=="pending"')
                    check('db-restore-cancel-no-stage', await probe.js('window.__T5_RESTORE') is None and not (root / 'pending-restore.sqlite').exists())
                    await probe.js('window.__T5_RESTORE="pending";window.__TAURI__.core.invoke("restore_db").then(v=>window.__T5_RESTORE=v,e=>window.__T5_RESTORE={error:String(e)});true')
                    await asyncio.sleep(.5)
                    tap(65,60)
                    await asyncio.sleep(.3)
                    tap(230,84)
                    await asyncio.sleep(.08)
                    tap(230,84)
                    await asyncio.sleep(.3)
                    tap(350,84)
                    tap(1045,799)
                    await asyncio.sleep(.5)
                    second_confirm = subprocess.check_output(['xwininfo','-root','-tree'],env=env,text=True)
                    check('db-restore-confirm-reopened', '"Restore from backup"' in second_confirm)
                    tap(485,215)
                    await probe.until('window.__T5_RESTORE!=="pending"')
                    restore_result = await probe.js('window.__T5_RESTORE')
                    check('db-restore-staged', restore_result == str(backup_path) and (root / 'pending-restore.sqlite').exists())
        await probe.js('document.querySelector("#settings-close").click();true')
        check('desktop-settings-close', await probe.js('document.querySelector("#settings-overlay").classList.contains("hidden")'))
        if picker:
            app.terminate(); app.wait(timeout=10)
            with (out / 'desktop-app-restart.log').open('w') as log:
                app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log)
            for _ in range(100):
                if not (root / 'pending-restore.sqlite').exists(): break
                assert app.poll() is None
                await asyncio.sleep(.1)
            with sqlite3.connect(db) as conn:
                restored_write = conn.execute("SELECT value FROM settings WHERE key='mcp.write_enabled'").fetchone()[0]
                restored_integrity = conn.execute('PRAGMA integrity_check').fetchone()[0]
            backups = list(root.glob('fixture.sqlite.bak-*'))
            check('db-restore-applied-on-restart', restored_write == 'true' and restored_integrity == 'ok' and bool(backups) and not (root / 'pending-restore.sqlite').exists(),
                  {'integrity':restored_integrity,'rollbackFiles':len(backups)})
        results['display'] = display
        results['desktopSha256'] = hashlib.sha256(Path('target/debug/rustrss-desktop').read_bytes()).hexdigest()
        results['fixturePath'] = str(db)
        results['passed'] = True
        (out / 'desktop-results.json').write_text(json.dumps(results, ensure_ascii=False, indent=2) + '\n')
        print(json.dumps({'checks': len(results['checks']), 'out': str(out)}), flush=True)
    finally:
        if app and app.poll() is None:
            app.terminate(); app.wait(timeout=10)
        xvfb.terminate(); xvfb.wait(timeout=10)

if __name__ == '__main__': asyncio.run(main())
