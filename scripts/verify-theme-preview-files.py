"""Model-free MCP file contract probe, including the real 600-second TTL.
Run from repository root with distro Python (Pillow installed) after cargo build.
Uses an isolated SQLite fixture and Xvfb; never changes the user's profile.
Evidence is retained at the printed path; no copied PNGs are needed.
"""
import json
import os
from pathlib import Path
import socket
import sqlite3
import stat
import subprocess
import tempfile
import time
import urllib.request
from PIL import Image


def main():
    subprocess.run(['df', '-h', '.'], check=True)
    root = Path(tempfile.mkdtemp(prefix='rustrss-preview-files-'))
    print(f'Evidence: {root}', flush=True)
    dbpath = root / 'fixture.sqlite'
    subprocess.run(['target/debug/examples/theme_fixture', str(dbpath)], check=True)
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    with sqlite3.connect(dbpath) as db:
        for key, value in [('mcp.enabled', 'true'), ('mcp.port', str(port)),
                           ('mcp.token', 'fixture-read'), ('mcp.write_token', 'fixture-write'),
                           ('mcp.write_enabled', 'true')]:
            db.execute('INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES(?,?,0)', (key, value))
    runtime = root / 'runtime'
    runtime.mkdir(mode=0o700)
    env = dict(os.environ, RUSTSS_DB=str(dbpath), RUSTSS_LOG_STDOUT='1',
               RUSTSS_AI_KEY='isolated-fixture-placeholder', XDG_DATA_HOME=str(root / 'data'),
               XDG_RUNTIME_DIR=str(runtime), GDK_GL='disable', GDK_SCALE='1')
    for key in ['GDK_BACKEND', 'WAYLAND_DISPLAY', 'EGL_PLATFORM']:
        env.pop(key, None)
    app = xvfb = None
    report = {'checks': [], 'samples': [], 'captures': []}
    tracked = []

    def call(name, args, token='fixture-write'):
        request = urllib.request.Request(f'http://127.0.0.1:{port}/mcp',
            data=json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
                             'params': {'name': name, 'arguments': args}}).encode(),
            headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json',
                     'Accept': 'application/json, text/event-stream'})
        with urllib.request.urlopen(request, timeout=15) as response:
            return json.load(response)['result']

    def value(reply):
        data = reply['structuredContent']
        assert data['ok'], data
        return data

    def capture(name, args):
        started = time.monotonic()
        reply = call(name, args)
        data = value(reply)
        assert all(item['type'] == 'text' for item in reply['content'])
        path = Path(data['image_path'])
        assert path.is_absolute() and data['image_mime_type'] == 'image/png'
        assert stat.S_IMODE(path.stat().st_mode) == 0o600
        assert stat.S_IMODE(path.parent.stat().st_mode) == 0o700
        assert path.stat().st_size == data['image_bytes'] <= 2 * 1024**2
        with Image.open(path) as image:
            image.load()
            assert image.format == 'PNG' and list(image.size) == data['capture']['pixel_size']
        assert data['capture']['freshness_marker_verified']
        assert 590 < (data['image_expires_at_ms'] / 1000 - time.time()) <= 600
        assert path not in [item[0] for item in tracked]
        tracked.append((path, started, data['image_expires_at_ms'] / 1000))
        report['captures'].append(data)
        return data

    def finish(preview, action):
        return value(call('finish_theme_preview', {'preview_id': preview['preview_id'],
            'expected_preview_revision': preview['preview_revision'], 'action': action}))

    def config():
        with sqlite3.connect(dbpath) as db:
            row = db.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone()
            return json.loads(row[0]) if row else None

    try:
        r, w = os.pipe()
        xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(w), '-screen', '0', '1400x1000x24'],
                                pass_fds=(w,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os.close(w)
        with os.fdopen(r) as pipe:
            env['DISPLAY'] = ':' + pipe.readline().strip()
        log = root / 'desktop.log'
        with log.open('w') as output:
            app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=output, stderr=output)
        deadline = time.monotonic() + 30
        while 'loaded feeds=' not in log.read_text():
            assert app.poll() is None and time.monotonic() < deadline, 'desktop boot failed'
            time.sleep(.1)
        capabilities = value(call('get_theme', {}, 'fixture-read'))['capabilities']['preview']
        assert capabilities['image_delivery'] == 'local_file' and capabilities['inline_images'] is False
        assert call('preview_theme', {'base_revision': 0, 'patch': {}}, 'fixture-read')['isError']
        p = capture('preview_theme', {'base_revision': 0, 'patch': {'light_preset': 'paper'}, 'scene': 'article'})
        again = capture('capture_theme_preview', {'preview_id': p['preview_id'], 'expected_preview_revision': p['preview_revision']})
        assert again['config_hash'] == p['config_hash'] and again['preview_revision'] == p['preview_revision']
        assert config() is None
        saved = finish(again, 'save')
        assert saved['saved_revision'] == 1 and finish(again, 'save') == saved
        baseline = config()
        assert baseline['current']['revision'] == 1 and baseline['current']['light_preset'] == 'paper'
        p = capture('preview_theme', {'base_revision': 1, 'patch': {'light_preset': 'slate'}})
        finish(p, 'cancel')
        assert config() == baseline and all(path.exists() for path, _, _ in tracked)
        report['checks'] += ['read_token_rejected', 'png_dimensions_and_permissions', 'unique_recapture',
                             'temporary_no_write', 'idempotent_save', 'cancel_no_write', 'finish_retains_files']
        print('Contract checks passed; waiting for real 600s expiration without further MCP requests.', flush=True)
        deadline = time.monotonic() + 620
        next_sample = 0
        while True:
            now = time.monotonic()
            assert app.poll() is None, 'desktop exited during TTL wait'
            remaining = []
            for path, started, expires in tracked:
                exists = path.exists()
                # Monotonic lower bound avoids treating wall-clock adjustments as early deletion.
                if now - started < 600:
                    assert exists, f'file deleted early: {path}'
                if exists:
                    remaining.append(path)
            if now >= next_sample:
                rss = next(line for line in Path(f'/proc/{app.pid}/status').read_text().splitlines() if line.startswith('VmRSS:'))
                disk = os.statvfs('.')
                report['samples'].append({'elapsed_seconds': round(now - tracked[0][1], 2),
                    'desktop_rss': rss, 'disk_available_bytes': disk.f_bavail * disk.f_frsize,
                    'remaining_files': len(remaining)})
                subprocess.run(['df', '-h', '.'], check=True)
                print(json.dumps(report['samples'][-1]), flush=True)
                next_sample = now + 60
            if not remaining:
                break
            assert now < deadline, 'expired files not reaped'
            time.sleep(.5)
        assert config() == baseline
        report['checks'].append('real_600s_expiration_without_requests')
        report['elapsed_seconds'] = round(time.monotonic() - tracked[0][1], 2)
        report['limitations'] = ['Xvfb only; no model image understanding asserted',
                                 'RSS is desktop process only, not WebKit subprocess tree']
        report['passed'] = True
        print(json.dumps({'passed': True, 'checks': report['checks'], 'elapsed_seconds': report['elapsed_seconds']}), flush=True)
    finally:
        (root / 'results.json').write_text(json.dumps(report, indent=2))
        for child in [app, xvfb]:
            if child and child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait(timeout=10)
        # SIGTERM does not exercise normal Tauri shutdown; remove only this probe's PNGs.
        for path, _, _ in tracked:
            path.unlink(missing_ok=True)
        for directory in {path.parent for path, _, _ in tracked}:
            if directory.exists():
                directory.rmdir()


if __name__ == '__main__':
    main()
