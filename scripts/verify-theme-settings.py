"""T4 production editor + isolated core IPC. Bounded three-frame Xvfb probe."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

subprocess.run(['df', '-h', '.'], check=True)
root = Path(tempfile.mkdtemp(prefix='rustrss-theme-settings-'))
env = dict(os.environ)
r, w = os.pipe()
xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(w), '-screen', '0', '1400x1000x24'], pass_fds=(w,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
os.close(w)
with os.fdopen(r) as pipe:
    env['DISPLAY'] = ':' + pipe.readline().strip()
for key in ('GDK_BACKEND', 'WAYLAND_DISPLAY', 'EGL_PLATFORM'):
    env.pop(key, None)
runtime = root / 'runtime'
runtime.mkdir(mode=0o700)
env.update(GDK_GL='disable', GDK_SCALE='1', XDG_RUNTIME_DIR=str(runtime))
try:
    result = subprocess.run(['target/debug/examples/theme_ui', str(root / 'captures'), '--settings'], env=env, capture_output=True, text=True, timeout=110)
    (root / 'run.log').write_text(result.stdout + result.stderr)
    report = json.loads((root / 'captures/results.json').read_text())
    assert result.returncode == 0 and not report.get('error'), report
    print(json.dumps({'output': str(root), 'checks': len(report['checks']), 'captures': len(report['captures'])}))
finally:
    xvfb.terminate()
    xvfb.wait(timeout=10)
    print('Evidence:', root)
