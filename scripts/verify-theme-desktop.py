"""Exercise actual rebuilt desktop IPC, theme picker, sliders and restart on a new fixture."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time
from PIL import Image

root = Path(tempfile.mkdtemp(prefix='rustrss-theme-desktop-'))
dbpath = root / 'fixture.sqlite'
subprocess.run(['target/debug/examples/theme_fixture', str(dbpath)], check=True)
read_fd, write_fd = os.pipe()
xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '1240x900x24'], pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
os.close(write_fd)
with os.fdopen(read_fd) as pipe:
    display = ':' + pipe.readline().strip()
runtime = root / 'runtime'
runtime.mkdir(mode=0o700)
env = dict(os.environ, DISPLAY=display, GDK_GL='disable', GDK_SCALE='1', XDG_RUNTIME_DIR=str(runtime),
           XDG_DATA_HOME=str(root / 'data'), RUSTSS_DB=str(dbpath), RUSTSS_LOG_STDOUT='1', RUSTSS_AI_KEY='isolated-fixture-placeholder')
# Discard session backend hints only for this isolated Xvfb child.
for key in ('GDK_BACKEND', 'WAYLAND_DISPLAY', 'EGL_PLATFORM'):
    env.pop(key, None)
app = None

def run(*args):
    return subprocess.check_output(args, env=env, text=True, timeout=10).strip()
def click(x, y):
    run('xdotool', 'mousemove', str(x), str(y), 'click', '1')
    time.sleep(0.35)
def shot(name):
    run('xdotool', 'mousemove', '1230', '810')
    run('xdotool', 'windowsize', window, '1239', '820')
    run('xdotool', 'windowsize', window, '1240', '820')
    time.sleep(0.25)
    run('import', '-window', window, str(root / name))
def config():
    with sqlite3.connect(dbpath) as db:
        row = db.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone()
        return json.loads(row[0]) if row else None
def start(name):
    global app, window
    logfile = root / name
    with logfile.open('w') as output:
        app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=output, stderr=output)
    for _ in range(100):
        if 'loaded feeds=' in logfile.read_text(): break
        if app.poll() is not None: raise RuntimeError(logfile.read_text())
        time.sleep(0.1)
    else: raise RuntimeError('desktop did not finish loading: ' + logfile.read_text())
    window = run('xdotool', 'search', '--onlyvisible', '--name', '^RustRss$').splitlines()[0]
    run('xdotool','windowfocus',window)
def stop():
    if app and app.poll() is None:
        app.terminate()
        try: app.wait(timeout=10)
        except subprocess.TimeoutExpired: app.kill(); app.wait(timeout=10)
try:
    start('first.log')
    assert config() is None, 'startup must not materialize legacy theme'
    click(330, 120)
    shot('initial-article.png')
    click(1080, 25)
    click(960, 298)
    shot('preset-menu.png')
    click(920, 368)
    shot('paper-settings.png')
    first = config()
    assert first['current']['light_preset'] == 'paper', first
    assert first['current']['dark_preset'] == 'paper', first
    assert first['current']['revision'] == 1, first
    click(985, 666)
    after = config()
    assert after['current']['overrides']['typography']['read_size'] > 18, after
    assert after['current']['revision'] == 2, after
    run('xdotool', 'key', 'Escape')
    shot('paper-article.png')
    stop()
    start('restart.log')
    assert config() == after, 'restart must not rewrite saved theme'
    click(330, 120)
    shot('restarted-paper.png')
    image = Image.open(root / 'restarted-paper.png').convert('RGB')
    assert image.getpixel((1200, 730)) == (255, 253, 247), image.getpixel((1200,730))
    report = {'output':str(root), 'preset':'paper', 'revision':after['current']['revision'], 'read_size':after['current']['overrides']['typography']['read_size'], 'restart_preserved':True}
    (root / 'results.json').write_text(json.dumps(report, indent=2))
    print(json.dumps(report))
finally:
    stop()
    xvfb.terminate()
    xvfb.wait(timeout=10)
    print('Evidence:',root, flush=True)
