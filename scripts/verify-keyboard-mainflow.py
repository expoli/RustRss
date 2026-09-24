"""键盘主流程闭环（T1 / 任务 201400c7）：真实会话注入按键 + 每步 UI 断言 + 库回读。

用法：python3 scripts/verify-keyboard-mainflow.py [证据目录]

串联（AC2）：移动 → 打开 → 返回 → 切视图(只看未读) → 搜索 → 标记已读 → 刷新 → **全部标记已读**；
并断言 `?` 帮助一览与实现一致（AC3：帮助里必须提到 `A`，且启动自检 `shortcut selftest ok`）。

关键点：`A` 是本次新增的实现（此前「全部标记已读」只有菜单入口，键盘 switch 里没有），
断言必须落到**库回读**（当前视图未读数归零），不能只看状态栏文案。
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

OUT_DIR = Path(sys.argv[1]) if len(sys.argv) > 1 else None
FEED_XML = (b"<?xml version='1.0'?><rss version='2.0'><channel><title>Keyboard Fixture</title>"
            b"<item><guid>kb-1</guid><title>refreshed by r key</title>"
            b"<description>body from local feed</description></item></channel></rss>")


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


class FeedHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header('Content-Type', 'application/rss+xml')
        self.send_header('Content-Length', str(len(FEED_XML)))
        self.end_headers()
        self.wfile.write(FEED_XML)

    def log_message(self, *_args):
        pass


def read_count(db_path):
    with sqlite3.connect(db_path) as db:
        return db.execute('SELECT COUNT(*) FROM entries WHERE read = 1').fetchone()[0]


def total_count(db_path):
    with sqlite3.connect(db_path) as db:
        return db.execute('SELECT COUNT(*) FROM entries').fetchone()[0]


async def main():
    report = {'checked_at': datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'), 'steps': []}
    with tempfile.TemporaryDirectory(prefix='rustrss-keyboard-') as temporary:
        root = Path(temporary)
        db_path = root / 'fixture.sqlite'
        subprocess.run(['target/debug/examples/theme_fixture', str(db_path)], check=True)
        feed_server = ThreadingHTTPServer(('127.0.0.1', 0), FeedHandler)
        threading.Thread(target=feed_server.serve_forever, daemon=True).start()
        with sqlite3.connect(db_path) as db:
            db.execute('UPDATE entries SET read = 0')
            db.execute("INSERT INTO feeds(url,title,created_at) VALUES(?,?,1)",
                       (f'http://127.0.0.1:{feed_server.server_port}/feed.xml', '键盘夹具源'))
            db.execute("UPDATE settings SET value='false' WHERE key='refresh.on_start'")
            db.execute("UPDATE settings SET value='off' WHERE key='refresh.interval_minutes'")
            db.execute("UPDATE settings SET value='false' WHERE key='list.hide_read'")
        before_total = total_count(db_path)

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
                app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log,
                                       start_new_session=True)
            window = None
            for _ in range(200):
                out = subprocess.run(['xdotool', 'search', '--onlyvisible', '--name', '^RustRss$'],
                                     env=env, capture_output=True, text=True).stdout.split()
                if out:
                    window = out[0]
                    break
                assert app.poll() is None, log_path.read_text()[-800:]
                await asyncio.sleep(.2)
            assert window, '未找到主窗口（xdotool search）'
            subprocess.run(['xdotool', 'windowfocus', window], env=env, check=True, timeout=10)

            probe = native_probe.Probe({'root': str(root), 'inspector_port': inspector_port})
            for _ in range(300):
                try:
                    await probe.js('1')
                    break
                except (OSError, IndexError):
                    await asyncio.sleep(.2)
            await probe.until("!!document.querySelector('#entries li[data-id]')")

            def key(value):
                subprocess.run(['xdotool', 'key', '--clearmodifiers', value], env=env, check=True, timeout=10)

            async def step(name, js, expect=True, wait=True):
                if wait:
                    await probe.until(js) if expect else await asyncio.sleep(.3)
                value = await probe.js(js)
                report['steps'].append({'step': name, 'expr': js[:90], 'value': value})
                return value

            # 1) 移动（j / k）
            active = "document.querySelector('#entries li.active')?.dataset.id||null"
            await probe.js(active)  # 初始可能还没选中行，先探一次
            key('j')
            await asyncio.sleep(.5)
            sel1 = await probe.js(active)
            assert sel1, 'j 应选中一行'
            key('j')
            await asyncio.sleep(.5)
            sel2 = await probe.js(active)
            assert sel2 and sel2 != sel1, f'j 应再往下走: {sel1} -> {sel2}'
            key('k')
            await asyncio.sleep(.5)
            sel3 = await probe.js(active)
            assert sel3 == sel1, f'k 应回到上一行: {sel3} != {sel1}'
            report['steps'].append({'step': 'move j/j/k', 'sel1': sel1, 'sel2': sel2, 'sel3_back': sel3})
            report['steps'].append({'step': 'move k back'})

            # 2) 打开（Enter 标已读）→ 3) 返回（Esc）
            key('Return')
            await probe.until("document.getElementById('reader').innerText.trim().length>40")
            opened = await probe.js("document.getElementById('reader').innerText.trim().length")
            assert opened > 0, 'Enter 应打开正文'
            assert read_count(db_path) >= 1, 'Enter 应把该篇标记已读（库回读）'
            report['steps'].append({'step': 'Enter opens article', 'article_chars': opened,
                                    'read_in_db': read_count(db_path)})
            key('Escape')
            await probe.until("!!document.querySelector('#entries li.active') && document.getElementById('entries').querySelectorAll('li[data-id]').length>0")
            report['steps'].append({'step': 'Escape returns to list (selection kept)'})

            # 4) 切视图：U = 只看未读（数据属性 / 文案变化）
            key('U')
            await asyncio.sleep(.6)
            unread_only = await probe.js("document.getElementById('btn-unread-only').getAttribute('aria-pressed')")
            assert unread_only == 'true', f'U 应打开只看未读: {unread_only!r}'
            report['steps'].append({'step': 'U unread-only toggle', 'aria_pressed': unread_only})
            key('U')
            await asyncio.sleep(.6)
            assert await probe.js("document.getElementById('btn-unread-only').getAttribute('aria-pressed')") == 'false', 'U 应能再关掉'
            await asyncio.sleep(.6)

            # 5) 搜索 → 6) Esc 清除
            total_rows = await probe.js("document.querySelectorAll('#entries li[data-id]').length")
            key('slash')
            await probe.until("document.activeElement && document.activeElement.id==='search'")
            subprocess.run(['xdotool', 'type', '--clearmodifiers', '--delay', '20', 'zzz-no-such-token-zzz'],
                           env=env, check=True, timeout=20)
            await asyncio.sleep(1.2)
            hits = await probe.js("document.querySelectorAll('#entries li[data-id]').length")
            report['steps'].append({'step': 'search filters list', 'rows_before': total_rows, 'rows_after': hits})
            assert hits == 0 and total_rows > 1, f'搜索无匹配词应过滤到 0 行（而非恒真断言）: {hits}/{total_rows}'
            key('Escape')
            await asyncio.sleep(.8)
            cleared = await probe.js("document.getElementById('search').value")
            restored = await probe.js("document.querySelectorAll('#entries li[data-id]').length")
            assert not cleared, f'Esc 应清空搜索框: {cleared!r}'
            assert restored == total_rows, f'Esc 后应恢复全量列表: {restored} != {total_rows}'
            report['steps'].append({'step': 'Escape clears search and restores list'})

            # 7) 标记已读（u 切换当前篇）
            await probe.js("document.querySelector('#entries li[data-id]').click()")
            await asyncio.sleep(.4)
            before_read = read_count(db_path)
            key('u')
            await asyncio.sleep(.8)
            after_read = read_count(db_path)
            report['steps'].append({'step': 'u toggles read', 'read_before': before_read, 'read_after': after_read})
            assert after_read != before_read, f'u 应改变已读状态: {before_read} -> {after_read}'

            # 8) 刷新（r）：本地 feed 应被写入
            total_before = total_count(db_path)
            key('r')
            refreshed = 0
            for _ in range(150):
                refreshed = total_count(db_path)
                if refreshed > total_before:
                    break
                await asyncio.sleep(.2)
            report['steps'].append({'step': 'r refreshes', 'entries_before': total_before,
                                    'entries_after': refreshed})
            assert refreshed > total_before, 'r 应从本地 feed 抓回新条目'

            # 9) 全部标记已读（A，本次新增实现）：库回读未读归零
            key('A')
            await asyncio.sleep(1.5)
            unread_left = total_count(db_path) - read_count(db_path)
            report['steps'].append({'step': 'A marks all read', 'unread_left': unread_left})
            assert unread_left == 0, f'A 之后未读应为 0，实际 {unread_left}'

            # 10) ? 帮助：必须提到 A；启动自检的 selftest 行也必须在日志里
            key('question')
            await probe.until("document.getElementById('keyboard-help') && document.getElementById('keyboard-help').open")
            help_text = await probe.js("document.getElementById('keyboard-help').innerText")
            assert 'A' in help_text, f'帮助一览应包含 A 快捷键: {help_text[:200]}'
            # 自检行写在**数据目录**的日志文件里（stdout 只有日志终端镜像开启时才有）：
            # 按 README，日志目录跟随数据目录而不跟随 RUSTSS_DB。
            logs_dir = Path(env['XDG_DATA_HOME']) / 'rustrss' / 'logs'
            log_files = sorted(logs_dir.glob('*.log'), key=lambda f: f.stat().st_mtime)
            joined = log_path.read_text()
            if log_files:
                joined += log_files[-1].read_text()
            assert 'shortcut selftest ok' in joined, f'启动自检应报 shortcut selftest ok（日志目录 {logs_dir}）'
            report['steps'].append({'step': '? help lists A', 'help_chars': len(help_text)})
            report['checks'] = ['full keyboard chain (move/open/back/view/search/read/refresh/mark-all-read)',
                               'help dialog and the in-app shortcut selftest agree with the handler']
            # 不落像素证据：无 WM 的 Xvfb 里 `import -window root` 会抓到陈旧/错误的帧
            # （实测抓到了另一个探针的拒绝界面），而本探针的断言全是 DOM/DB 级，察觉不到。
            # 环境限制如实记录；要像素证据需真实桌面会话（见 README 的无头验证手册）。
            report['screenshot'] = None
            report['screenshot_note'] = ('无 WM 的 Xvfb 中 import 取帧不可靠（会拿到陈旧帧），'
                                         '故不作像素断言；本任务的证据为 DOM + SQLite 回读')
            report['fixture'] = {'entries_before': before_total, 'entries_after': total_count(db_path)}
            report['evidence_file'] = write_evidence('keyboard-mainflow-results.json', report)
            print(json.dumps(report, ensure_ascii=False))
        finally:
            if app and app.poll() is None:
                app.terminate()
                try:
                    app.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    app.kill()
                    app.wait(timeout=5)
            xvfb.terminate()
            xvfb.wait(timeout=10)
            feed_server.shutdown()


asyncio.run(main())
