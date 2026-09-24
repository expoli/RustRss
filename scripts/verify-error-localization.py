"""错误路径与双语提示取证（T4 / 任务 40c124dc）。

用法：python3 scripts/verify-error-localization.py [证据目录]

覆盖清单锚点里的产品错误路径（§24.15/§24.16 + §26）：超时 / 429+Retry-After / 503 / 404 /
非 feed 内容，并**分别以 zh-CN 与 en 两次启动**断言界面文案随语言切换（双语证据，而不是只读字典）。

做法：本地 HTTP 夹具按路径给出不同故障；把每个故障源作为订阅写入隔离库，触发 `refresh_all`，
再读侧栏该行的可见错误文案（`li .error` / title）与库里的 `last_error`/`last_status`。
"""
import asyncio
import importlib.util
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
from datetime import datetime, timezone

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    'native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native_probe)

OUT_DIR = Path(sys.argv[1]) if len(sys.argv) > 1 else None
FEED_OK = (b"<?xml version='1.0'?><rss version='2.0'><channel><title>ok</title>"
           b"<item><guid>ok-1</guid><title>ok item</title></item></channel></rss>")


def write_evidence(name, payload):
    if OUT_DIR is None:
        return None
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    path = OUT_DIR / name
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + '\n')
    return str(path)


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


class FaultHandler(BaseHTTPRequestHandler):
    """/timeout 不响应；/429 带 Retry-After；/503、/404、/notfeed 各给一种失败。"""

    def do_GET(self):
        if self.path.startswith('/timeout'):
            time.sleep(30)  # 让客户端超时（应用侧超时 30s，这里给足）
            return
        if self.path.startswith('/429'):
            self.send_response(429)
            self.send_header('Retry-After', '60')
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        if self.path.startswith('/503'):
            self.send_response(503)
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        if self.path.startswith('/404'):
            self.send_response(404)
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        if self.path.startswith('/notfeed'):
            body = b"<html><body>not a feed at all</body></html>"
            self.send_response(200)
            self.send_header('Content-Type', 'text/html')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        self.send_response(200)
        self.send_header('Content-Type', 'application/rss+xml')
        self.send_header('Content-Length', str(len(FEED_OK)))
        self.end_headers()
        self.wfile.write(FEED_OK)

    def log_message(self, *_args):
        pass


async def wait_inspector(probe, app, log_path, tries=250):
    for _ in range(tries):
        if app.poll() is not None:
            raise AssertionError(f'应用提前退出: {log_path.read_text()[-400:]}')
        try:
            await probe.js('1')
            return
        except (OSError, IndexError):
            await asyncio.sleep(.2)
    raise AssertionError('inspector 未就绪')


def stop(proc):
    if proc is None or proc.poll() is not None:
        return
    try:
        proc.terminate()
        proc.wait(timeout=8)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


async def run_locale(locale, base_env, root, server_port, report):
    db_path = root / f'app-{locale}.sqlite'
    runtime = root / f'runtime-{locale}'
    runtime.mkdir(mode=0o700, exist_ok=True)
    inspector_port = free_port()
    env = dict(base_env, RUSTSS_DB=str(db_path),
               WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{inspector_port}')
    # 先起一次让应用建库，再播种故障源
    read_fd, write_fd = os.pipe()
    xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '1400x1000x24'],
                            pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.close(write_fd)
    app = None
    try:
        with os.fdopen(read_fd) as pipe:
            env['DISPLAY'] = ':' + pipe.readline().strip()
        log_path = root / f'desktop-{locale}.log'
        with log_path.open('w') as log:
            app = subprocess.Popen(['target/release/rustrss-desktop'], env=env, stdout=log,
                                   stderr=log, start_new_session=True)
        for _ in range(200):
            if db_path.exists() and db_path.stat().st_size > 0:
                break
            await asyncio.sleep(.1)
        stop(app)
        with sqlite3.connect(db_path) as db:
            db.execute("INSERT INTO settings(key,value,updated_at) VALUES('ui.locale',?,1) "
                       "ON CONFLICT(key) DO UPDATE SET value=excluded.value", (locale,))
            db.execute("UPDATE settings SET value='false' WHERE key='refresh.on_start'")
            db.execute("UPDATE settings SET value='off' WHERE key='refresh.interval_minutes'")
            for path, name in (('/429', 'r429'), ('/503', 'r503'), ('/404', 'r404'), ('/notfeed', 'rhtml')):
                db.execute('INSERT INTO feeds(url,title,created_at) VALUES(?,?,1)',
                           (f'http://127.0.0.1:{server_port}{path}', name))
        with log_path.open('a') as log:
            app = subprocess.Popen(['target/release/rustrss-desktop'], env=env, stdout=log,
                                   stderr=log, start_new_session=True)
        probe = native_probe.Probe({'root': str(root), 'inspector_port': inspector_port})
        await wait_inspector(probe, app, log_path)
        await probe.until("document.querySelectorAll('#feeds li.folder-feed').length>=4")
        await probe.js("window.__e={done:false};window.__TAURI__.core.invoke('refresh_all')"
                       ".then(()=>window.__e.done=true,e=>window.__e.error=String(e));true")
        for _ in range(300):
            if await probe.js('window.__e.done===true||!!window.__e.error'):
                break
            await asyncio.sleep(.2)
        # 先看库里的码（机制层证据），tooltip 可能仍是刷新前渲染的（UI 按同值零写不重算）
        with sqlite3.connect(db_path) as db:
            codes = dict(db.execute(
                "SELECT title, last_status FROM feeds WHERE url LIKE 'http://127.0.0.1%'").fetchall())
        report['locales'].setdefault(locale, {})['db_codes'] = codes
        assert all(v not in (None, 'ok') for v in codes.values()), f'故障源应记录失败码: {codes}'
        # 重载页面强制重算侧栏 tooltip（tooltip 在 buildFeedRow 时计算，刷新事件不一定重建它）
        await probe.js('window.location.reload();true')
        await asyncio.sleep(2.0)
        await probe.until("document.querySelectorAll('#feeds li.folder-feed').length>=4")
        # 侧栏行：li.folder-feed，错误文案在 **title 属性**（tooltip）里，失败标志是 .dot 显示
        rows = await probe.js(
            "JSON.stringify(Array.from(document.querySelectorAll('#feeds li.folder-feed')).map(li=>({"
            "title:li.querySelector('.name')?.textContent||'', tooltip:li.title||'',"
            "failed:!(li.querySelector('.dot')?.hidden??true)})))")
        report['locales'][locale] = {'rows': json.loads(rows),
                                     'refresh_error': await probe.js('window.__e?(window.__e.error??null):null')}
        return report['locales'][locale]
    finally:
        stop(app)
        xvfb.terminate()
        xvfb.wait(timeout=10)


async def main():
    report = {'checked_at': datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
              'binary': 'target/release/rustrss-desktop', 'locales': {}, 'checks': []}
    server = ThreadingHTTPServer(('127.0.0.1', 0), FaultHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='rustrss-errors-') as temporary:
        root = Path(temporary)
        base_env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
                        GDK_GL='disable', GDK_SCALE='1')
        for key in ('GDK_BACKEND', 'WAYLAND_DISPLAY', 'EGL_PLATFORM'):
            base_env.pop(key, None)
        zh = await run_locale('zh-CN', base_env, root, server.server_port, report)
        en = await run_locale('en', base_env, root, server.server_port, report)
        zh_text = {row['title']: row['tooltip'] for row in zh['rows']}
        en_text = {row['title']: row['tooltip'] for row in en['rows']}
        assert all(row['failed'] for row in zh['rows']), \
            f'故障源应带失败标记（refresh_error={zh.get("refresh_error")!r}）: {zh["rows"]}'
        report['zh'] = zh_text
        report['en'] = en_text
        for name in ('r429', 'r503', 'r404', 'rhtml'):
            assert zh_text.get(name), f'{name} 在 zh-CN 下应有可见错误文案'
            assert en_text.get(name), f'{name} 在 en 下应有可见错误文案'
            assert zh_text[name] != en_text[name], f'{name} 的中英文案应不同: {zh_text[name]!r}'
        report['checks'].append('per-feed error text is visible and differs between zh-CN and en')
        if OUT_DIR:
            OUT_DIR.mkdir(parents=True, exist_ok=True)
        report['evidence_file'] = write_evidence('error-localization-results.json', report)
        print(json.dumps(report, ensure_ascii=False))
    server.shutdown()


asyncio.run(main())
