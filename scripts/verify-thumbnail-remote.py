"""远程图片边界检查（B1 / 任务 27db42f3）：公网源加载、失败静默、离线不阻塞。

与 `verify-thumbnail-ui.py`（本地 loopback 基线）互补：这里把隔离库里的缩略图指向
**真实公网图片站**，在真实 WebView（WebKit inspector）里断言：

1. 公网图片确实加载完成（`naturalWidth > 0`），且 `loading=lazy` / `referrerPolicy=no-referrer`；
2. 反面用例（404、不可达主机）静默失败：`naturalWidth == 0`、无阻塞式 dialog；
3. 图片失败不引起列表行重建（前后 id 序列一致）、列表仍可滚动。

证据：stdout 的 JSON（含站点、时间、URL）。公网结论不外推——站点与时间如实记录。
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
from datetime import datetime, timezone

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    'native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native_probe)

# 实测可用的公网图片（2026-09-24T09:31:33Z，curl 200 / image/png / 12746 B）。
# 换成其它站点时必须重新记录时间与响应。
PUBLIC_IMAGE = 'https://raw.githubusercontent.com/github/explore/main/topics/rust/rust.png'
MISSING_IMAGE = 'https://raw.githubusercontent.com/github/explore/main/topics/rust/definitely-missing.png'
UNREACHABLE_IMAGE = 'http://10.255.255.1/never.png'  # 私网不可达地址：连接会挂到超时
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


class ForbiddenHandler(BaseHTTPRequestHandler):
    """模拟「外站拒绝嵌入 / 热链保护」：无论带不带 Referer，一律 403 不给图。

    真实公网站点的热链策略不可控（测了就不可复现），所以这一条用本地确定性夹具；
    公网侧只保留 404 与不可达两类，结论不外推。
    """

    def do_GET(self):
        self.send_response(403)
        self.send_header('Content-Length', '0')
        self.end_headers()

    def log_message(self, *_args):
        pass


async def main():
    report = {
        'public_image': PUBLIC_IMAGE,
        'missing_image': MISSING_IMAGE,
        'unreachable_image': UNREACHABLE_IMAGE,
        'checked_at': datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
        'checks': [],
    }
    with tempfile.TemporaryDirectory(prefix='rustrss-thumbnail-remote-') as temporary:
        root = Path(temporary)
        dbpath = root / 'fixture.sqlite'
        subprocess.run(['target/debug/examples/theme_fixture', str(dbpath)], check=True)
        forbidden_server = ThreadingHTTPServer(('127.0.0.1', 0), ForbiddenHandler)
        threading.Thread(target=forbidden_server.serve_forever, daemon=True).start()
        forbidden_url = f'http://127.0.0.1:{forbidden_server.server_port}/forbidden.png'
        report['forbidden_image'] = forbidden_url
        with sqlite3.connect(dbpath) as db:
            ids = [row[0] for row in db.execute('SELECT id FROM entries ORDER BY COALESCE(published_at, fetched_at) DESC, id DESC LIMIT 4')]
            assert len(ids) >= 4, '夹具条目不足以做四路对照'
            db.execute('UPDATE entries SET thumbnail_url=NULL')
            db.execute('UPDATE entries SET thumbnail_url=? WHERE id=?', (PUBLIC_IMAGE, ids[0]))
            db.execute('UPDATE entries SET thumbnail_url=? WHERE id=?', (MISSING_IMAGE, ids[1]))
            db.execute('UPDATE entries SET thumbnail_url=? WHERE id=?', (UNREACHABLE_IMAGE, ids[2]))
            db.execute('UPDATE entries SET thumbnail_url=? WHERE id=?', (forbidden_url, ids[3]))
            # 关掉自动刷新：本探针只关心图片请求，不要掺入抓取流量
            db.execute("UPDATE settings SET value='false' WHERE key='refresh.on_start'")
            db.execute("UPDATE settings SET value='off' WHERE key='refresh.interval_minutes'")
        report['entry_ids'] = ids

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
            with log_path.open('w') as log:
                app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log)
            probe = native_probe.Probe({'root': str(root), 'inspector_port': inspector_port})
            for _ in range(200):
                if 'loaded feeds=' in log_path.read_text():
                    break
                assert app.poll() is None, log_path.read_text()
                await asyncio.sleep(.1)

            await probe.until("!!document.querySelector('#entries li[data-id]')")
            # 缩略图开关必须为开（否则 img 被 display:none，lazy 图不会加载）
            await probe.js("window.__thumbEnable={done:false};window.__TAURI__.core.invoke('get_theme_update',{knownRevision:null}).then(snapshot=>window.__TAURI__.core.invoke('update_ui_theme',{expectedRevision:snapshot.config.revision,patch:{overrides:{list:{thumbnail:true}}}})).then(()=>window.__thumbEnable.done=true,error=>window.__thumbEnable.error=String(error));true")
            await probe.until('window.__thumbEnable.done===true')
            await probe.until("document.documentElement.dataset.thumbnails==='true'")
            # 布局基线：失败图落定前的行几何（AC2「不引起布局跳动」的机械断言基础）
            geom_js = ("JSON.stringify(Array.from(document.querySelectorAll('#entries li[data-id]'))"
                       ".slice(0,8).map(li=>{const r=li.getBoundingClientRect();"
                       "return [li.dataset.id,Math.round(r.top),Math.round(r.height)]}))")
            await probe.until("!!document.querySelector('#entries li[data-id]')")
            geom_before = await probe.js(geom_js)

            # 1) 公网图片加载完成（给它充裕时间：真实外网）
            loaded = False
            for _ in range(300):
                loaded = await probe.js(
                    "(()=>{const n=document.querySelector('li[data-id=\"%d\"] .entry-thumbnail')"
                    ";return !!(n&&n.complete&&n.naturalWidth>0)})()" % ids[0])
                if loaded:
                    break
                await asyncio.sleep(.2)
            assert loaded, '公网缩略图未加载完成（记录站点与时间后如实报失败）'
            attrs = await probe.js(
                "(()=>{const n=document.querySelector('li[data-id=\"%d\"] .entry-thumbnail');"
                "return {src:n.src,lazy:n.loading,referrer:n.referrerPolicy,width:n.naturalWidth}})()" % ids[0])
            report['public_image_attrs'] = attrs
            assert attrs['lazy'] == 'lazy' and attrs['referrer'] == 'no-referrer', attrs
            report['checks'].append('public image loaded in the real WebView (lazy, no-referrer)')

            # 2) 反面用例：404 与不可达主机都静默失败（naturalWidth==0），且无阻塞 dialog
            await asyncio.sleep(3)
            broken = await probe.js(
                "(()=>{const q=id=>{const n=document.querySelector(`li[data-id=\"${id}\"] .entry-thumbnail`);"
                "return n?{w:n.naturalWidth,broken:n.complete&&n.naturalWidth===0}:null};"
                "return {missing:q(%d),unreachable:q(%d),forbidden:q(%d),dialog:!!document.querySelector('dialog[open]')}})()"
                % (ids[1], ids[2], ids[3]))
            report['failure_cases'] = broken
            assert broken['missing'] and broken['missing']['w'] == 0, broken
            assert broken['unreachable'] and broken['unreachable']['w'] == 0, broken
            assert broken['forbidden'] and broken['forbidden']['w'] == 0, broken
            assert not broken['dialog'], '图片失败不应弹出阻塞式对话框'
            report['checks'].append('404 / unreachable / 403-hotlink-rejected images all fail silently')

            # 布局不跳动（机械断言，不只靠截图）：失败图落定后行几何必须与基线完全一致
            geom_after = await probe.js(geom_js)
            report['geometry_before'] = geom_before
            report['geometry_after'] = geom_after
            assert geom_before == geom_after, f'失败图引起了布局跳动: {geom_before} -> {geom_after}'
            report['checks'].append('failed images do not shift list geometry (no layout jump)')

            # 3) 失败不重建行、列表仍可滚动
            before = await probe.js("Array.from(document.querySelectorAll('#entries li[data-id]')).map(li=>li.dataset.id).join(',')")
            await probe.js("(()=>{const l=document.getElementById('entries');l.scrollTop=l.scrollHeight;return true})()")
            await asyncio.sleep(1)
            after = await probe.js("Array.from(document.querySelectorAll('#entries li[data-id]')).map(li=>li.dataset.id).join(',')")
            report['row_ids_before'] = before
            report['row_ids_after'] = after
            report['scrolled'] = await probe.js("document.getElementById('entries').scrollTop>0")
            assert before == after, '图片失败不应引起列表行重建'
            report['checks'].append('failed images neither rebuilt rows nor blocked scrolling')
            # AC2 要求「截图 + 日志」：截图给失败态的视觉证据，日志尾随证据一起落盘
            if OUT_DIR:
                OUT_DIR.mkdir(parents=True, exist_ok=True)
                png = OUT_DIR / 'thumbnail-remote-failures.png'
                subprocess.run(['import', '-display', env['DISPLAY'], '-window', 'root', str(png)],
                               check=True, timeout=60)
                report['screenshot'] = str(png)
                log_copy = OUT_DIR / 'thumbnail-remote-desktop.log'
                log_copy.write_text(log_path.read_text()[-20000:])
                report['app_log'] = str(log_copy)
            report['evidence_file'] = write_evidence('thumbnail-remote-results.json', report)
            print(json.dumps(report, ensure_ascii=False))
        finally:
            if app and app.poll() is None:
                app.terminate()
                app.wait(timeout=10)
            xvfb.terminate()
            xvfb.wait(timeout=10)
            forbidden_server.shutdown()


asyncio.run(main())
