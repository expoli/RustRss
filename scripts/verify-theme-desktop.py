"""Exercise actual rebuilt desktop IPC, theme picker, sliders and restart on a new fixture."""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import time
from PIL import Image

subprocess.run(['df', '-h', '.'], check=True)
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
env = dict(os.environ, HOME=str(root / 'home'), DISPLAY=display, GDK_GL='disable', GDK_SCALE='1', XDG_RUNTIME_DIR=str(runtime),
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
    click(330, 150)
    assert 'renderReader id=' in (root/'first.log').read_text()
    renders = (root/'first.log').read_text().count('renderReader id=')
    click(1080, 25)
    shot('appearance.png')
    # Active-tab pixels verify actual keyboard navigation through all seven panes.
    navigation = []
    run('xdotool','key','Home')
    for index in range(7):
        if index: run('xdotool','key','Down')
        time.sleep(.25)
        shot('nav.png')
        image = Image.open(root/'nav.png').convert('RGB')
        assert image.getpixel((300,120+45*index)) == (226,232,244), (index,image.getpixel((300,120+45*index)))
        navigation.append(index)
    run('xdotool','key','Home')
    time.sleep(.2)
    run('xdotool','key','shift+Tab')
    run('xdotool','key','Tab')
    run('xdotool','key','End')
    time.sleep(.25)
    shot('general-keyboard.png')
    assert Image.open(root/'general-keyboard.png').convert('RGB').getpixel((300,390)) == (226,232,244), 'focus did not wrap back into tab navigation'
    run('xdotool','key','Escape')
    click(610,160)
    shot('aa-before.png')
    click(690,280)
    run('xdotool','key','ctrl+a')
    run('xdotool','type','23')
    run('xdotool','key','Tab')
    time.sleep(.25)
    assert config() is None, 'Aa draft must not write'
    click(545,740)
    after = config()
    assert after['current']['overrides']['typography']['read_size'] == 23, after
    assert after['current']['revision'] == 1, after
    run('xdotool','key','Escape')
    shot('saved-reader.png')
    assert (root/'first.log').read_text().count('renderReader id=') == renders, 'theme save rebuilt article'
    stop()
    start('restart.log')
    assert config() == after, 'restart must not rewrite saved theme'
    report = {'output':str(root), 'keyboard_categories':len(navigation), 'focus_wrap':True, 'aa_draft_no_write':True, 'read_size':23, 'revision':1, 'article_renders':[renders,renders], 'restart_preserved':True}
    (root/'results.json').write_text(json.dumps(report,indent=2))
    print(json.dumps(report))
finally:
    stop()
    xvfb.terminate()
    xvfb.wait(timeout=10)
    print('Evidence:',root,flush=True)
