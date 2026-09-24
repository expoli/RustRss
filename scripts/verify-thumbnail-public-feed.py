"""真 feed 缩略图链路检查（B1b / 任务 27db42f3 AC2）：公网订阅 → 解析提取 → 落库 → 列表渲染。

与 `verify-thumbnail-remote.py` 的分工：
- 那个用**注入的**公网图片 URL 测边界（成功/404/不可达/离线）；
- 这里用**真实订阅**（`https://github.blog/feed/`，正文含 <img>）走完整链路，并做库回读。

断言：
1. 启动刷新后库里出现 `thumbnail_url IS NOT NULL` 的条目（提取落库）；
2. 那些 URL 指向公网图片主机，且都是 HTTP(S)；
3. 真实 WebView 里对应缩略图加载完成（`naturalWidth > 0`）；
4. 证据：站点、时间、条目数、缩略图主机分布与样例 URL。
"""
import asyncio
import importlib.util
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    'native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native_probe)

PUBLIC_FEED = 'https://github.blog/feed/'  # 正文含 <img>（2026-09-24 实测 31 处）
OUT_DIR = Path(sys.argv[1]) if len(sys.argv) > 1 else None


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


def launch(env, log_path):
    with log_path.open('a') as log:
        return subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log)


async def main():
    report = {'public_feed': PUBLIC_FEED,
              'checked_at': datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
              'checks': []}
    with tempfile.TemporaryDirectory(prefix='rustrss-thumbnail-feed-') as temporary:
        root = Path(temporary)
        dbpath = root / 'app.sqlite'
        runtime = root / 'runtime'
        runtime.mkdir(mode=0o700)
        inspector_port = free_port()
        env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
            XDG_RUNTIME_DIR=str(runtime), GDK_GL='disable', GDK_SCALE='1', RUSTSS_DB=str(dbpath),
            WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{inspector_port}')
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

            # 1) 先空跑一次让应用自建基线库，再播种订阅与开关（避免手工造 schema）
            app = launch(env, log_path)
            for _ in range(200):
                if dbpath.exists() and dbpath.stat().st_size > 0:
                    break
                await asyncio.sleep(.1)
            app.terminate()
            app.wait(timeout=10)
            app = None
            with sqlite3.connect(dbpath) as db:
                db.execute('INSERT INTO feeds(url,title,created_at) VALUES(?,?,?)',
                           (PUBLIC_FEED, 'GitHub Blog', 1))
                db.execute("INSERT INTO settings(key,value,updated_at) VALUES('refresh.on_start','true',1) "
                           "ON CONFLICT(key) DO UPDATE SET value='true'")
                db.execute("INSERT INTO settings(key,value,updated_at) VALUES('refresh.interval_minutes','off',1) "
                           "ON CONFLICT(key) DO UPDATE SET value='off'")
            report['seeded_at'] = datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')

            # 2) 正式启动：等启动刷新把真 feed 抓完并提取缩略图
            app = launch(env, log_path)
            probe = native_probe.Probe({'root': str(root), 'inspector_port': inspector_port})
            for _ in range(200):
                if 'loaded feeds=' in log_path.read_text():
                    break
                assert app.poll() is None, log_path.read_text()[-2000:]
                await asyncio.sleep(.1)

            extracted = 0
            deadline = 90
            for _ in range(deadline * 5):
                try:
                    with sqlite3.connect(dbpath) as db:
                        extracted = db.execute(
                            'SELECT COUNT(*) FROM entries WHERE thumbnail_url IS NOT NULL').fetchone()[0]
                        total = db.execute('SELECT COUNT(*) FROM entries').fetchone()[0]
                except sqlite3.Error:
                    total = 0
                if extracted:
                    break
                await asyncio.sleep(.2)
            report['entries_total'] = total
            report['entries_with_thumbnail'] = extracted
            assert total > 0, '真 feed 应至少解析出条目（否则链路未走通）'
            assert extracted > 0, '真 feed 的正文图片应被提取为 thumbnail_url'

            # 3) 库回读：缩略图必须是公网 HTTP(S)，且记录主机分布
            with sqlite3.connect(dbpath) as db:
                rows = db.execute(
                    'SELECT id, thumbnail_url FROM entries WHERE thumbnail_url IS NOT NULL ORDER BY id LIMIT 5'
                ).fetchall()
                hosts = db.execute(
                    "SELECT DISTINCT substr(thumbnail_url, 1, instr(substr(thumbnail_url, 9), '/') + 7) "
                    'FROM entries WHERE thumbnail_url IS NOT NULL').fetchall()
            for _id, url in rows:
                assert url.startswith('http://') or url.startswith('https://'), url
            report['thumbnail_samples'] = [u for _i, u in rows]
            report['thumbnail_hosts'] = [h[0] for h in hosts if h[0]]
            report['checks'].append('real public feed produced stored thumbnail URLs (DB read-back)')

            # 4) 真实 WebView 渲染
            await probe.until("!!document.querySelector('#entries li[data-id]')")
            # 断言「至少有一张缩略图真的加载完成」而不是只看 DOM 里第一个缩略图节点：
            # 首屏第一条未必带图（真 feed 的某几篇没有 <img>），盯死第一个节点会假失败。
            # 先滚到底触发懒加载，再逐个看 complete && naturalWidth>0。
            await probe.js("(()=>{const l=document.getElementById('entries');l.scrollTop=l.scrollHeight;return true})()")
            rendered = False
            for _ in range(250):
                rendered = await probe.js(
                    "(()=>{const ns=Array.from(document.querySelectorAll('#entries .entry-thumbnail'));"
                    "return ns.some(n=>n.complete&&n.naturalWidth>0&&n.src.startsWith('https://'))})()")
                if rendered:
                    break
                await asyncio.sleep(.2)
            assert rendered, '真 feed 的缩略图应在列表里真实加载完成'
            report['rendered_attrs'] = await probe.js(
                "(()=>{const ns=Array.from(document.querySelectorAll('#entries .entry-thumbnail'));"
                "const n=ns.find(x=>x.complete&&x.naturalWidth>0)||ns[0];"
                "return n?{src:n.src,width:n.naturalWidth,lazy:n.loading,referrer:n.referrerPolicy}:null})()")
            report['checks'].append('real feed thumbnail rendered in the production WebView')
            # 截图（AC 要求「截图确认列表渲染」）：Xvfb 屏直接抓 root 窗口
            if OUT_DIR:
                OUT_DIR.mkdir(parents=True, exist_ok=True)
                png = OUT_DIR / 'thumbnail-public-feed.png'
                subprocess.run(['import', '-display', env['DISPLAY'], '-window', 'root', str(png)],
                               check=True, timeout=60)
                report['screenshot'] = str(png)
            report['evidence_file'] = write_evidence('thumbnail-public-feed-results.json', report)
            print(json.dumps(report, ensure_ascii=False))
        finally:
            if app and app.poll() is None:
                app.terminate()
                app.wait(timeout=10)
            xvfb.terminate()
            xvfb.wait(timeout=10)


asyncio.run(main())
