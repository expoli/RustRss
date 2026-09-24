"""HTTPS 代理 CONNECT 隧道成功路径（T3 / 任务 fa49a263 AC3）。

用法：python3 scripts/verify-https-connect-tunnel.py [证据目录]

做法：起一个**只做 CONNECT 转发**的本地代理（记录目标 host:port 并双向透传字节），
把隔离实例的 `network.proxy` 设成它，再订阅一个**真实 HTTPS** feed 并触发刷新：
- 断言代理日志里出现 `CONNECT github.blog:443`（隧道真的被用上，而不是直连成功）；
- 断言库里真的抓到了条目（隧道端到端可用，不是只握了手）；
- 控制组：把代理指到一个**关闭的端口**再刷一次 → 失败可读、**已缓存条目仍在**（可继续导航）。

⚠ 如实标注外部依赖：企业证书 / 需要认证的代理未验（需要真实网关与凭据），
本探针只覆盖「本地 CONNECT 隧道 + 真实公网 HTTPS 目标」这一条成功路径。
"""
import asyncio
import importlib.util
import json
import os
from pathlib import Path
import select
import socket
import socketserver
import sqlite3
import subprocess
import sys
import tempfile
import threading
from datetime import datetime, timezone

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    'native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native_probe)

HTTPS_FEED = 'https://github.blog/feed/'
OUT_DIR = Path(sys.argv[1]) if len(sys.argv) > 1 else None
CONNECT_LOG = []


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


class ConnectProxy(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True


class ConnectHandler(socketserver.BaseRequestHandler):
    def handle(self):
        request = b''
        while b'\r\n\r\n' not in request:
            chunk = self.request.recv(4096)
            if not chunk:
                return
            request += chunk
        line = request.split(b'\r\n', 1)[0].decode('latin-1')
        if not line.startswith('CONNECT '):
            self.request.sendall(b'HTTP/1.1 405 Method Not Allowed\r\n\r\n')
            return
        host_port = line.split(' ', 2)[1]
        host, _, port = host_port.rpartition(':')
        CONNECT_LOG.append(host_port)
        try:
            upstream = socket.create_connection((host, int(port)), timeout=15)
        except OSError as error:
            self.request.sendall(b'HTTP/1.1 502 Bad Gateway\r\n\r\n')
            CONNECT_LOG.append(f'FAILED {host_port}: {error}')
            return
        self.request.sendall(b'HTTP/1.1 200 Connection Established\r\n\r\n')
        sockets = [self.request, upstream]
        try:
            while True:
                readable, _, _ = select.select(sockets, [], [], 30)
                if not readable:
                    break
                for source in readable:
                    data = source.recv(65536)
                    if not data:
                        return
                    (upstream if source is self.request else self.request).sendall(data)
        finally:
            upstream.close()


def entries(db_path):
    with sqlite3.connect(db_path) as db:
        return db.execute('SELECT COUNT(*) FROM entries').fetchone()[0]


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


async def main():
    binary = Path('target/release/rustrss-desktop')
    report = {'binary': str(binary),
              'binary_mtime': datetime.fromtimestamp(binary.stat().st_mtime).isoformat(timespec='seconds'),'https_feed': HTTPS_FEED,
              'checked_at': datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'), 'checks': []}
    proxy = ConnectProxy(('127.0.0.1', 0), ConnectHandler)
    proxy_port = proxy.server_address[1]
    threading.Thread(target=proxy.serve_forever, daemon=True).start()
    report['proxy'] = f'http://127.0.0.1:{proxy_port}'
    with tempfile.TemporaryDirectory(prefix='rustrss-tunnel-') as temporary:
        root = Path(temporary)
        db_path = root / 'app.sqlite'
        runtime = root / 'runtime'
        runtime.mkdir(mode=0o700)
        inspector_port = free_port()
        env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
                   XDG_RUNTIME_DIR=str(runtime), GDK_GL='disable', GDK_SCALE='1',
                   RUSTSS_DB=str(db_path), WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{inspector_port}')
        for key in ('GDK_BACKEND', 'WAYLAND_DISPLAY', 'EGL_PLATFORM'):
            env.pop(key, None)
        read_fd, write_fd = os.pipe()
        xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '1400x1000x24'],
                                pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os.close(write_fd)
        app = None
        try:
            with os.fdopen(read_fd) as pipe:
                env['DISPLAY'] = ':' + pipe.readline().strip()
            log_path = root / 'desktop.log'
            with log_path.open('w') as log:
                boot = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log,
                                        start_new_session=True)
            for _ in range(200):
                if db_path.exists() and db_path.stat().st_size > 0:
                    break
                await asyncio.sleep(.1)
            stop(boot)
            with sqlite3.connect(db_path) as db:
                db.execute('INSERT INTO feeds(url,title,created_at) VALUES(?,?,1)',
                           (HTTPS_FEED, 'GitHub Blog (via tunnel)'))
                db.execute("INSERT INTO settings(key,value,updated_at) VALUES('network.proxy',?,1) "
                           "ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                           (json.dumps({'mode': 'custom', 'url': f'http://127.0.0.1:{proxy_port}',
                                        'no_proxy': ''}),))
                db.execute("UPDATE settings SET value='false' WHERE key='refresh.on_start'")
                db.execute("UPDATE settings SET value='off' WHERE key='refresh.interval_minutes'")

            with log_path.open('a') as log:
                app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log,
                                       start_new_session=True)
            probe = native_probe.Probe({'root': str(root), 'inspector_port': inspector_port})
            await wait_inspector(probe, app, log_path)
            await probe.js("window.__t={done:false};window.__TAURI__.core.invoke('refresh_all')"
                           ".then(r=>window.__t.result=r,e=>window.__t.error=String(e));true")
            got = 0
            for _ in range(300):
                got = entries(db_path)
                if got:
                    break
                await asyncio.sleep(.2)
            report['entries_after_tunnel'] = got
            report['connect_log'] = list(CONNECT_LOG)
            report['refresh_error'] = await probe.js('(window.__t.error)??null')
            assert any('github.blog:443' in entry for entry in CONNECT_LOG), \
                f'代理日志应出现 CONNECT github.blog:443（否则是直连成功而非隧道）: {CONNECT_LOG}'
            assert got > 0, '经隧道应真的抓回条目'
            report['checks'].append('refresh over a local CONNECT tunnel reached the public HTTPS feed')

            # 控制组：代理指到关闭的端口 → 失败可读、已缓存条目仍在
            with sqlite3.connect(db_path) as db:
                db.execute("UPDATE settings SET value=? WHERE key='network.proxy'",
                           (json.dumps({'mode': 'custom', 'url': 'http://127.0.0.1:1',
                                        'no_proxy': ''}),))
            await probe.js("window.__t2={done:false};window.__TAURI__.core.invoke('refresh_all')"
                           ".then(r=>window.__t2.result=r,e=>window.__t2.error=String(e));true")
            for _ in range(150):
                done = await probe.js('window.__t2.done===true||!!window.__t2.error')
                if done:
                    break
                await asyncio.sleep(.2)
            failed = await probe.js('JSON.stringify(window.__t2.error??window.__t2.result??null)')
            after_failure = entries(db_path)
            report['control_failure'] = {'result': failed, 'entries_still_cached': after_failure}
            assert 'connection_error' in failed, f'代理不可用应给出可读的 connection_error: {failed}'
            assert after_failure >= got, '代理不可用时已缓存条目必须仍在'
            report['checks'].append('unreachable proxy fails readably and cached entries survive')
            # 像素证据不可靠（见搜索探针的说明）：无 WM 的 Xvfb 会取到陈旧帧
            report['screenshot'] = None
            report['screenshot_note'] = ('无 WM 的 Xvfb 中 import 取帧不可靠（实测不同探针产出逐字节相同的 PNG），'
                                         '故不作像素证据；本任务证据为代理日志 + 抓取计数 + DOM/库回读')
            report['evidence_file'] = write_evidence('https-connect-tunnel-results.json', report)
            print(json.dumps(report, ensure_ascii=False))
        finally:
            stop(app)
            xvfb.terminate()
            xvfb.wait(timeout=10)
            proxy.shutdown()


asyncio.run(main())
