"""旧库拒绝的运行时验收（T4 / 任务 317029ec）：拒绝提示 + 库不被写 + 备份重建路径。

用法：python3 scripts/verify-legacy-refusal-ui.py [证据目录]

覆盖：
1. 隔离实例用**旧链形状**的库启动 → 出现拒绝遮罩（标题/正文取 i18n 双语 key，按钮齐全）；
2. 拒绝后旧库**一个字节都没变**（SHA256 + `application_id`/`user_version` 回读）；
3. 截图存证；
4. 「备份 → 重建」路径：把旧库另存为备份（含 SHA256 与可读性校验）、再启动 → 应用建出**新基线库**
   （`application_id` 魔数 + `user_version=1`），能加订阅并成功刷新一次本地 feed。

⚠ 如实记录的边界：退出面板上的「导出 OPML」会打开**原生保存对话框**（GTK），无 WM 的 Xvfb
里驱动不了它 —— 本探针只断言按钮存在与命令已注册；导出行为本身由 core 测试覆盖
（`tests/opml.rs::export_read_only_rescues_feeds_from_a_legacy_database`：只读、零写入、无 WAL 边车）。
"""
import asyncio
import hashlib
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

REPO = Path(__file__).resolve().parent.parent
OUT_DIR = Path(sys.argv[1]) if len(sys.argv) > 1 else None
FEED_XML = (b"<?xml version='1.0'?><rss version='2.0'><channel><title>Rebuilt Fixture</title>"
            b"<item><guid>r1</guid><title>first item after rebuild</title></item></channel></rss>")


def write_evidence(name, payload):
    if OUT_DIR is None:
        return None
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    path = OUT_DIR / name
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + '\n')
    return str(path)


def sha256(path):
    h = hashlib.sha256()
    h.update(Path(path).read_bytes())
    return h.hexdigest()


def pragmas(path):
    conn = sqlite3.connect(f'file:{path}?mode=ro', uri=True)
    try:
        return {
            'application_id': conn.execute('PRAGMA application_id').fetchone()[0],
            'user_version': conn.execute('PRAGMA user_version').fetchone()[0],
        }
    finally:
        conn.close()


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


def legacy_db(path):
    """旧链形状（v1 表 + 用户数据，无 application_id）：正是被拒绝的那一类库。"""
    conn = sqlite3.connect(path)
    conn.executescript(
        """
        CREATE TABLE folders (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, position INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE feeds (id INTEGER PRIMARY KEY, url TEXT NOT NULL UNIQUE, title TEXT NOT NULL,
            site_url TEXT, description TEXT, language TEXT,
            folder_id INTEGER REFERENCES folders(id) ON DELETE SET NULL, etag TEXT, last_modified TEXT,
            last_fetched_at INTEGER, last_status TEXT, last_error TEXT, created_at INTEGER NOT NULL);
        INSERT INTO folders(id,name,position) VALUES(1,'旧分组',0);
        INSERT INTO feeds(id,url,title,folder_id,created_at) VALUES(1,'https://legacy.invalid/feed.xml','旧源',1,1);
        PRAGMA user_version=13;
        """
    )
    conn.commit()
    conn.close()


async def run_app(env_base, db_path, root, script, expect_overlay):
    """起一个隔离实例，返回 (app, xvfb, probe, log_path)。调用方负责 stop。"""
    runtime = root / f'runtime-{script}'
    runtime.mkdir(mode=0o700, exist_ok=True)
    inspector_port = free_port()
    env = dict(env_base, RUSTSS_DB=str(db_path),
               WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{inspector_port}')
    read_fd, write_fd = os.pipe()
    xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '1400x1000x24'],
                            pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.close(write_fd)
    with os.fdopen(read_fd) as pipe:
        env['DISPLAY'] = ':' + pipe.readline().strip()
    log_path = root / f'desktop-{script}.log'
    with log_path.open('w') as log:
        app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log,
                               start_new_session=True)
    probe = native_probe.Probe({'root': str(root), 'inspector_port': inspector_port})
    return app, xvfb, probe, log_path, env


async def wait_inspector(probe, app, log_path, tries=200):
    """等 WebKit inspector 真的在听（应用启动是异步的；直接打会 Connection refused 而死）。"""
    last = None
    for _ in range(tries):
        if app.poll() is not None:
            raise AssertionError(f'应用提前退出，日志：{log_path.read_text()[-500:]}')
        try:
            await probe.js('1')
            return
        except (OSError, IndexError) as e:
            last = e
            await asyncio.sleep(.2)
    raise AssertionError(f'inspector 未就绪: {last}')


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
    report = {'checked_at': datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'), 'checks': []}
    with tempfile.TemporaryDirectory(prefix='rustrss-legacy-refusal-') as temporary:
        root = Path(temporary)
        db_path = root / 'rustrss.sqlite'
        legacy_db(db_path)
        before_hash, before_pragmas = sha256(db_path), pragmas(db_path)
        report['legacy'] = {'sha256_before': before_hash, **before_pragmas}

        base_env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
                        GDK_GL='disable', GDK_SCALE='1')
        base_env['XDG_RUNTIME_DIR'] = str(root / 'runtime-base')
        Path(base_env['XDG_RUNTIME_DIR']).mkdir(mode=0o700, exist_ok=True)
        for key in ('GDK_BACKEND', 'WAYLAND_DISPLAY', 'EGL_PLATFORM'):
            base_env.pop(key, None)

        # ---- 1) 拒绝态 ----
        app, xvfb, probe, log_path, env = await run_app(base_env, db_path, root, 'refusal', True)
        try:
            await wait_inspector(probe, app, log_path)
            await probe.until("!!document.getElementById('startup-refusal') && !document.getElementById('startup-refusal').hidden")
            state = await probe.js(
                "(()=>{const o=document.getElementById('startup-refusal');"
                "const q=s=>{const e=o.querySelector(s);return e?e.textContent.trim():null};"
                "return {title:q('h2'),body:q('p'),path:q('code'),"
                "export:q('#startup-refusal-export'),quit:q('#startup-refusal-quit'),"
                "visible:getComputedStyle(o).display!=='none'}})()")
            report['overlay'] = state
            assert state['visible'], state
            assert state['title'] and state['body'], f'遮罩文案应非空（i18n key）: {state}'
            assert state['export'] and state['quit'], f'两个动作按钮应存在: {state}'
            assert state['path'] and state['path'].endswith('rustrss.sqlite'), state
            report['checks'].append('refusal overlay rendered with i18n copy and both actions')
            if OUT_DIR:
                OUT_DIR.mkdir(parents=True, exist_ok=True)
                png = OUT_DIR / 'legacy-refusal-overlay.png'
                subprocess.run(['import', '-display', env['DISPLAY'], '-window', 'root', str(png)],
                               check=True, timeout=60)
                report['screenshot'] = str(png)
            after_hash, after_pragmas = sha256(db_path), pragmas(db_path)
            report['legacy'].update({'sha256_after': after_hash, **after_pragmas})
            assert before_hash == after_hash, '拒绝不得写旧库（SHA256 必须一致）'
            assert before_pragmas == after_pragmas, '拒绝不得改 PRAGMA'
            assert not (root / 'rustrss.sqlite-wal').exists(), '拒绝不得建 WAL 边车'
            report['checks'].append('legacy database byte-identical after refusal')
            report['app_log_tail'] = log_path.read_text()[-400:]
        finally:
            stop(app)
            xvfb.terminate()
            xvfb.wait(timeout=10)

        # ---- 2) 备份 → 重建 → 新库可用 ----
        backup = root / 'RustRss-backup-legacy.sqlite'
        subprocess.run(['cp', str(db_path), str(backup)], check=True)
        backup_hash = sha256(backup)
        report['backup'] = {'path': str(backup), 'sha256': backup_hash,
                            'readable_pragmas': pragmas(backup)}
        with sqlite3.connect(f'file:{backup}?mode=ro', uri=True) as b:
            assert b.execute('SELECT COUNT(*) FROM feeds').fetchone()[0] == 1, '备份应可读且含订阅'
        os.replace(db_path, root / 'rustrss.sqlite.moved-aside')  # 用户按提示重建：移开旧库
        feed_server = ThreadingHTTPServer(('127.0.0.1', 0), FeedHandler)
        threading.Thread(target=feed_server.serve_forever, daemon=True).start()
        app, xvfb, probe, log_path, env = await run_app(base_env, db_path, root, 'rebuilt', False)
        try:
            await wait_inspector(probe, app, log_path)
            await probe.until("!document.getElementById('startup-refusal')||document.getElementById('startup-refusal').hidden")
            for _ in range(150):
                if db_path.exists() and db_path.stat().st_size > 0:
                    break
                await asyncio.sleep(.2)
            fresh = pragmas(db_path)
            report['rebuilt'] = fresh
            assert fresh['application_id'] == 0x52535331, f'重建后应带基线魔数: {fresh}'
            assert fresh['user_version'] == 1, f'重建后 user_version 应为 1: {fresh}'
            report['checks'].append('rebuild produced a baseline database (marker + version 1)')

            # 加订阅并刷新一次（用本地 feed，避免外网）
            with sqlite3.connect(db_path) as db:
                db.execute('INSERT INTO feeds(url,title,created_at) VALUES(?,?,?)',
                           (f'http://127.0.0.1:{feed_server.server_port}/feed.xml', '重建后订阅', 1))
            await probe.js("window.__r={done:false};window.__TAURI__.core.invoke('refresh_all')"
                           ".then(()=>window.__r.done=true,e=>window.__r.error=String(e));true")
            got = 0
            for _ in range(200):
                with sqlite3.connect(db_path) as db:
                    got = db.execute('SELECT COUNT(*) FROM entries').fetchone()[0]
                if got:
                    break
                await asyncio.sleep(.2)
            report['rebuilt_entries'] = got
            assert got > 0, '重建后的库应能成功刷新一次（本地 feed）'
            report['checks'].append('rebuilt database fetched a feed successfully')
        finally:
            stop(app)
            xvfb.terminate()
            xvfb.wait(timeout=10)
            feed_server.shutdown()

        report['evidence_file'] = write_evidence('legacy-refusal-runtime-results.json', report)
        print(json.dumps(report, ensure_ascii=False))


asyncio.run(main())
