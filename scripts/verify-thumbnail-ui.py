"""Isolated production WebView check for list thumbnail loading and theme toggle."""
import asyncio
import base64
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

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native_probe)

PNG = base64.b64decode('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jZ2sAAAAASUVORK5CYII=')
IMAGE_REQUESTS = []


class ImageHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        IMAGE_REQUESTS.append({'path': self.path, 'referer': self.headers.get('Referer')})
        self.send_response(200)
        self.send_header('Content-Type', 'image/png')
        self.send_header('Content-Length', str(len(PNG)))
        self.send_header('Cache-Control', 'no-store')
        self.end_headers()
        self.wfile.write(PNG)

    def log_message(self, *_args):
        pass


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


async def main():
    subprocess.run(['df', '-h', '.'], check=True)
    with tempfile.TemporaryDirectory(prefix='rustrss-thumbnail-ui-') as temporary:
        root = Path(temporary)
        dbpath = root / 'fixture.sqlite'
        subprocess.run(['target/debug/examples/theme_fixture', str(dbpath)], check=True)
        image_server = ThreadingHTTPServer(('127.0.0.1', 0), ImageHandler)
        threading.Thread(target=image_server.serve_forever, daemon=True).start()
        image_url = f'http://127.0.0.1:{image_server.server_port}/cover.png'
        with sqlite3.connect(dbpath) as db:
            db.execute('UPDATE entries SET thumbnail_url=?', (image_url,))
            db.execute("UPDATE settings SET value='false' WHERE key='refresh.on_start'")
            db.execute("UPDATE settings SET value='off' WHERE key='refresh.interval_minutes'")

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
        report = {'checks': [], 'image_url': image_url}
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
            await probe.until("!!document.querySelector('#entries .entry-thumbnail')")
            await probe.until("document.querySelector('#entries .entry-thumbnail').naturalWidth===1")
            assert IMAGE_REQUESTS and IMAGE_REQUESTS[0]['path'] == '/cover.png' and IMAGE_REQUESTS[0]['referer'] is None, IMAGE_REQUESTS
            check = await probe.js("(()=>{const image=document.querySelector('#entries .entry-thumbnail');window.__thumbnailNode=image;return image.src===" + json.dumps(image_url) +
                "&&image.loading==='lazy'&&image.referrerPolicy==='no-referrer'&&document.documentElement.dataset.thumbnails==='true'&&getComputedStyle(image).display!=='none'})()")
            assert check, 'visible list image should be loaded lazily without a referrer under the enabled theme option'
            report['image_request'] = IMAGE_REQUESTS[0]
            report['checks'].append('actual WebView loaded the local HTTP image and emitted expected lazy/referrer/theme attributes')
            await probe.js("window.__thumbnailUpdate={done:false};window.__TAURI__.core.invoke('get_theme_update',{knownRevision:null}).then(snapshot=>window.__TAURI__.core.invoke('update_ui_theme',{expectedRevision:snapshot.config.revision,patch:{overrides:{list:{thumbnail:false}}}})).then(()=>window.__thumbnailUpdate.done=true,error=>window.__thumbnailUpdate.error=String(error));true")
            await probe.until('window.__thumbnailUpdate.done===true')
            await probe.until("document.documentElement.dataset.thumbnails==='false'")
            toggled = await probe.js("getComputedStyle(window.__thumbnailNode).display==='none'&&window.__thumbnailNode.isConnected&&window.__thumbnailNode===document.querySelector('#entries .entry-thumbnail')")
            assert toggled, 'theme option should hide the existing thumbnail node without rebuilding it'
            report['checks'].append('theme option hides the same existing image node')
            print(json.dumps(report))
        finally:
            if app and app.poll() is None:
                app.terminate()
                app.wait(timeout=10)
            xvfb.terminate()
            xvfb.wait(timeout=10)
            image_server.shutdown()


asyncio.run(main())
