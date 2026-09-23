"""Run the rebuilt desktop on an isolated fixture and private Xvfb display.
Interactive driver: start, use xdotool with printed DISPLAY, then terminate owned PIDs.
"""
import os
from pathlib import Path
import subprocess
import sqlite3
import time

root = Path('target/verification-followups').resolve()
root.mkdir(exist_ok=True)
if not (root / 'fixture.sqlite').is_file():
    raise SystemExit('Create the isolated fixture with scope_counts init first')
with sqlite3.connect(root / 'fixture.sqlite') as db:
    for key, value in [('refresh.interval_minutes', 'off'), ('refresh.on_start', 'false'), ('mcp.enabled', 'false')]:
        db.execute('INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES(?,?,0)', (key, value))
read_fd, write_fd = os.pipe()
display_log = (root / 'xvfb.log').open('w')
xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '1240x900x24'],
                        pass_fds=(write_fd,), stdout=display_log, stderr=display_log)
os.close(write_fd)
with os.fdopen(read_fd) as pipe:
    display = ':' + pipe.readline().strip()
env = dict(os.environ, DISPLAY=display, GDK_GL='disable', RUSTSS_LOG_STDOUT='1',
           XDG_DATA_HOME=str(root / 'data'), RUSTSS_DB=str(root / 'fixture.sqlite'),
           RUSTSS_AI_KEY='isolated-ui-verification-placeholder')
# Keep GTK from connecting to the user's Wayland session.
env.pop('WAYLAND_DISPLAY', None)
runtime = root / 'runtime'
runtime.mkdir(mode=0o700, exist_ok=True)
env['XDG_RUNTIME_DIR'] = str(runtime)
log = (root / 'desktop.log').open('w')
app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log)
(root / 'processes.txt').write_text(f'DISPLAY={display}\nXVFB_PID={xvfb.pid}\nAPP_PID={app.pid}\n')
print((root / 'processes.txt').read_text(), flush=True)
try:
    def run(*args):
        return subprocess.check_output(args, env=env, text=True, timeout=10).strip()
    def click(x, y):
        run('xdotool', 'mousemove', str(x), str(y), 'click', '1')
        time.sleep(0.6)
    def shot(name):
        run('xdotool', 'windowsize', window, '1239', '820')
        run('xdotool', 'windowsize', window, '1240', '820')
        time.sleep(0.2)
        run('import', '-window', 'root', str(root / name))
    for _ in range(30):
        time.sleep(0.3)
        if 'loaded feeds=' in (root / 'desktop.log').read_text(): break
    window = run('xdotool', 'search', '--onlyvisible', '--name', 'RustRss').splitlines()[0]
    run('xdotool', 'windowraise', window, 'windowfocus', window)
    click(110, 250)
    shot('folder.png')
    assert 'view=folder' in (root / 'desktop.log').read_text()
    assert 'view total folder#1 n=6000' in (root / 'desktop.log').read_text()
    click(21, 250)
    shot('folder-collapsed.png')
    click(70, 109)
    click(1080, 25)
    click(175, 153)
    shot('settings.png')
    click(420, 358)
    time.sleep(1)
    with sqlite3.connect(root / 'fixture.sqlite') as db:
        marked, outside = db.execute('SELECT sum(read AND starred), sum(read AND NOT starred) FROM entries').fetchone()
    assert (marked, outside) == (100, 0), (marked, outside)
    shot('starred-marked.png')
    print('PASS: folder total=6000; starred marked=100; outside marked=0', flush=True)
finally:
    if app.poll() is None:
        app.terminate()
        try: app.wait(timeout=10)
        except subprocess.TimeoutExpired:
            app.kill()
            app.wait(timeout=10)
    xvfb.terminate()
    xvfb.wait(timeout=10)
