"""Start an isolated native Wayland desktop for verify-theme-wayland.py.
Run with distro Python, keep this terminal open, Ctrl-C when finished.
No input authorization is requested by this launcher.
"""
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile
import time

assert os.environ.get('WAYLAND_DISPLAY'), 'Run from the logged-in Wayland session'
subprocess.run(['df', '-h', '.'], check=True)
root = Path(tempfile.mkdtemp(prefix='rustrss-t7-native-'))
def port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]
mcp, inspector = port(), port()
assert mcp != inspector
fixture = root / 'fixture.sqlite'
subprocess.run(['target/debug/examples/theme_fixture', str(fixture)], check=True)
with sqlite3.connect(fixture) as db:
    for key, value in [('mcp.enabled','true'), ('mcp.port',str(mcp)), ('mcp.token','fixture-read'), ('mcp.write_enabled','true'), ('mcp.write_token','fixture-write')]:
        db.execute('INSERT OR REPLACE INTO settings(key,value,updated_at) VALUES(?,?,0)', (key,value))
env = dict(os.environ, HOME=str(root/'home'), XDG_DATA_HOME=str(root/'data'), RUSTSS_DB=str(fixture), RUSTSS_LOG_STDOUT='1', RUSTSS_AI_KEY='isolated-fixture-placeholder', WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{inspector}')
env.pop('DISPLAY', None)
with (root/'desktop.log').open('w') as log:
    app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log)
info = dict(root=str(root), mcp_port=mcp, inspector_port=inspector, pid=app.pid)
(root/'instance.json').write_text(json.dumps(info))
print(f"Wait for desktop boot, then: /usr/bin/python3 scripts/verify-theme-wayland.py {root}/instance.json", flush=True)
try:
    while app.poll() is None:
        time.sleep(1)
except KeyboardInterrupt:
    pass
finally:
    if app.poll() is None:
        app.terminate()
        try: app.wait(timeout=10)
        except subprocess.TimeoutExpired: app.kill(); app.wait(timeout=10)
    print('Evidence retained:', root)
