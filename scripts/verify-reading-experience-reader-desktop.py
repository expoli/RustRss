"""Linux WebKitGTK reader keyboard check in an isolated virtual KWin session.

Run: dbus-run-session -- /usr/bin/python3 scripts/verify-reading-experience-reader-desktop.py OUT
KeyboardEvents are sent through WebKit Inspector to the real desktop WebView.
"""
import asyncio
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import sqlite3
import subprocess
import sys
import tempfile

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
    root = Path(tempfile.mkdtemp(prefix='rustrss-t4-reader-desktop-'))
    runtime = root / 'runtime'
    runtime.mkdir(mode=0o700)
    db = root / 'fixture.sqlite'
    shutil.copy2('/tmp/rustrss-t4-fixture.sqlite', db)
    env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
               XDG_RUNTIME_DIR=str(runtime), RUSTSS_DB=str(db), GDK_GL='disable',
               GDK_BACKEND='wayland', WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{free_port()}')
    env.pop('DISPLAY', None)
    env.pop('WAYLAND_DISPLAY', None)
    socket_name = 'wayland-rustrss-t4-reader'
    report = {'window': 'Linux WebKitGTK in virtual KWin Wayland', 'input': 'DOM KeyboardEvent via WebKit Inspector', 'checks': []}
    with (out / 'desktop-kwin.log').open('w') as log:
        kwin = subprocess.Popen(['kwin_wayland', '--virtual', '--socket', socket_name,
                                 '--width', '1600', '--height', '1000', '--no-lockscreen', '--no-global-shortcuts'],
                                env=env, stdout=log, stderr=subprocess.STDOUT)
    app = None
    try:
        for _ in range(200):
            if (runtime / socket_name).exists(): break
            assert kwin.poll() is None, 'KWin exited'
            await asyncio.sleep(.1)
        assert (runtime / socket_name).exists(), 'KWin socket not ready'
        env['WAYLAND_DISPLAY'] = socket_name
        with (out / 'desktop-app.log').open('w') as log:
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
        def check(value, name):
            assert value, name
            report['checks'].append(name)
        await probe.until("!!document.querySelector('#entries li[data-id=\"1\"] .title')")
        await probe.js("document.querySelector('#entries li[data-id=\"1\"] .title').click()")
        await probe.until("!!document.querySelector('#act-more')")
        await probe.js("document.querySelector('#act-more').click()")
        await probe.until("!!document.querySelector('#ctx-menu')")
        check(await probe.js("document.querySelector('#ctx-menu').getAttribute('role')==='menu'"), 'reader More exposes desktop menu role')
        check(await probe.js("document.activeElement===document.querySelector('#ctx-menu > button')"), 'first reader menu item receives focus')
        key = "document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:%s,bubbles:true,cancelable:true}))"
        await probe.js(key % json.dumps('ArrowDown'))
        check(await probe.js("document.activeElement===document.querySelectorAll('#ctx-menu > button')[1]"), 'ArrowDown moves reader menu focus')
        await probe.js(key % json.dumps('Escape'))
        check(await probe.js("!document.querySelector('#ctx-menu') && document.activeElement?.id==='act-more'"), 'Escape closes and restores More focus')
        with sqlite3.connect(db) as conn:
            before = conn.execute('SELECT read FROM entries WHERE id=1').fetchone()[0]
        check(before == 1, 'direct article opening marked read')
        await probe.js("document.querySelector('#act-more').click()")
        await probe.until("!!document.querySelector('#ctx-menu')")
        await probe.js("[...document.querySelectorAll('#ctx-menu button')].find(n=>n.textContent.includes('标为未读')).focus()")
        # Inspector KeyboardEvent cannot synthesize the browser's trusted button default action.
        await probe.js("document.activeElement.click()")
        await probe.until("!document.querySelector('#ctx-menu')")
        with sqlite3.connect(db) as conn:
            after = conn.execute('SELECT read FROM entries WHERE id=1').fetchone()[0]
        check(after == 0, 'focused mark-unread activation writes once')
        await probe.js("document.querySelector('#act-aa').click()")
        await probe.until("document.querySelector('#aa-dialog').open")
        check(await probe.js("document.querySelector('#aa-dialog').open && !!document.querySelector('#aa-editor [data-theme-field=\"typography.read_size\"]')"), 'Aa reading editor opens with named size field')
        await probe.js("document.querySelector('#aa-close').click()")
        check(await probe.js("!document.querySelector('#aa-dialog').open"), 'Aa Close button dismisses dialog')
        report['passed'] = True
        (out / 'desktop-results.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
        print(json.dumps({'checks': len(report['checks']), 'out': str(out)}), flush=True)
    finally:
        if app and app.poll() is None:
            app.terminate(); app.wait(timeout=10)
        kwin.terminate(); kwin.wait(timeout=10)

if __name__ == '__main__': asyncio.run(main())
