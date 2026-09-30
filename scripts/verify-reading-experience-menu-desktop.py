"""Check T2 keyboard menu behavior in an isolated Linux WebKitGTK window.

Run with: dbus-run-session -- /usr/bin/python3 scripts/verify-reading-experience-menu-desktop.py OUT
Uses virtual KWin and WebKit Inspector DOM key events; no user session or data.
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
import time

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
    root = Path(tempfile.mkdtemp(prefix='rustrss-t2-menu-desktop-'))
    runtime = root / 'runtime'
    runtime.mkdir(mode=0o700)
    db = root / 'fixture.sqlite'
    shutil.copy2('/tmp/rustrss-t2-fixture.sqlite', db)
    env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
               XDG_RUNTIME_DIR=str(runtime), RUSTSS_DB=str(db), GDK_GL='disable',
               GDK_BACKEND='wayland', WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{free_port()}')
    env.pop('DISPLAY', None)
    env.pop('WAYLAND_DISPLAY', None)
    socket_name = 'wayland-rustrss-t2-menu'
    report = {'window': 'Linux WebKitGTK in virtual KWin Wayland', 'input': 'DOM KeyboardEvent via WebKit Inspector', 'checks': []}
    with (out / 'kwin.log').open('w') as log:
        kwin = subprocess.Popen(['kwin_wayland', '--virtual', '--socket', socket_name,
                                 '--width', '1600', '--height', '1000', '--no-lockscreen', '--no-global-shortcuts'],
                                env=env, stdout=log, stderr=subprocess.STDOUT)
    app = None
    try:
        for _ in range(200):
            if (runtime / socket_name).exists():
                break
            assert kwin.poll() is None, 'KWin exited'
            await asyncio.sleep(.1)
        assert (runtime / socket_name).exists(), 'KWin socket not ready'
        env['WAYLAND_DISPLAY'] = socket_name
        with (out / 'desktop.log').open('w') as log:
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
        await probe.until("!!document.querySelector('#feeds li[data-feed-id=\"2\"] .row-more')")
        await probe.js("document.querySelector('#feeds li[data-feed-id=\"2\"] .row-more').click()")
        await probe.until("!!document.querySelector('#ctx-menu')")
        def check(value, name):
            assert value, name
            report['checks'].append(name)
        check(await probe.js("document.querySelector('#ctx-menu').getAttribute('role')==='menu'"), 'desktop menu opens from explicit row button')
        check(await probe.js("document.activeElement===document.querySelector('#ctx-menu > button')"), 'first menu item receives focus')
        key = "document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:%s,bubbles:true,cancelable:true}))"
        await probe.js(key % json.dumps('ArrowDown'))
        check(await probe.js("document.activeElement===document.querySelectorAll('#ctx-menu > button')[1]"), 'ArrowDown moves focus')
        for _ in range(8):
            if await probe.js("document.activeElement.textContent.includes('刷新间隔')"):
                break
            await probe.js(key % json.dumps('ArrowDown'))
        check(await probe.js("document.activeElement.textContent.includes('刷新间隔')"), 'interval parent reachable by keyboard')
        await probe.js(key % json.dumps('ArrowRight'))
        await probe.until("!!document.querySelector('.ctx-submenu')")
        check(await probe.js("document.activeElement.closest('.ctx-submenu')!==null"), 'ArrowRight opens submenu and focuses its first item')
        check(await probe.js("document.querySelector('.ctx-submenu [aria-checked=\"true\"]')?.textContent.includes('跟随全局')"), 'submenu exposes current value')
        await probe.js(key % json.dumps('ArrowLeft'))
        check(await probe.js("!document.querySelector('.ctx-submenu') && document.activeElement.textContent.includes('刷新间隔')"), 'ArrowLeft returns to parent')
        await probe.js(key % json.dumps('Escape'))
        check(await probe.js("!document.querySelector('#ctx-menu') && document.activeElement===document.querySelector('#feeds li[data-feed-id=\"2\"] .row-more')"), 'Escape closes menu and restores trigger focus')
        await probe.js("document.querySelector('#tags li[data-tag-id=\"1\"] .row-more').click()")
        await probe.until("!!document.querySelector('#ctx-menu')")
        check(await probe.js("[...document.querySelectorAll('#ctx-menu > button')].some(b=>b.textContent.includes('标签下移'))"), 'tag touch sort action is also on desktop menu')
        await probe.js(key % json.dumps('Escape'))
        await probe.js("document.querySelector('#feeds li[data-folder-id=\"1\"] .row-more').click()")
        await probe.until("!!document.querySelector('#ctx-menu')")
        check(await probe.js("[...document.querySelectorAll('#ctx-menu > button')].some(b=>b.textContent.includes('删除'))"), 'folder danger action is on desktop menu')
        await probe.js(key % json.dumps('Escape'))
        with sqlite3.connect(db) as conn:
            unchanged = conn.execute('SELECT refresh_interval_minutes FROM feeds WHERE id=2').fetchone()[0] is None
        check(unchanged, 'open and close caused no feed write')
        report['passed'] = True
        (out / 'results.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
        print(json.dumps({'checks': len(report['checks']), 'out': str(out)}), flush=True)
    finally:
        if app and app.poll() is None:
            app.terminate()
            app.wait(timeout=10)
        kwin.terminate()
        kwin.wait(timeout=10)


if __name__ == '__main__':
    asyncio.run(main())
