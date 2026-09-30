from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import json

COUNT = Path('/tmp/rustrss-t4-ai-count.txt')
PAGE_COUNT = Path('/tmp/rustrss-t4-page-count.txt')
MODE = Path('/tmp/rustrss-t4-ai-mode.txt')
PAGE_MODE = Path('/tmp/rustrss-t4-page-mode.txt')

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('Content-Length', '0')))
        try:
            task = 'translate' if '翻译'.encode() in body or b'translate' in body.lower() else 'summary'
        except Exception:
            task = 'unknown'
        count = int(COUNT.read_text()) if COUNT.exists() else 0
        COUNT.write_text(str(count + 1))
        if MODE.exists() and MODE.read_text().strip() == 'fail':
            self.send_response(503)
            self.end_headers()
            self.wfile.write(b'{"error":"synthetic outage"}')
            return
        result = json.dumps({'response': f'Synthetic {task} result #{count + 1}', 'done': True}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(result)))
        self.end_headers()
        self.wfile.write(result)

    def do_GET(self):
        count = int(PAGE_COUNT.read_text()) if PAGE_COUNT.exists() else 0
        PAGE_COUNT.write_text(str(count + 1))
        if self.path == '/page' or (self.path == '/missing' and PAGE_MODE.exists() and PAGE_MODE.read_text().strip() == 'recover'):
            page = ('<html><body><main><h1>Synthetic full text</h1><article>'
                    + '<p>Offline fixture paragraph for reader validation.</p>' * 50
                    + '</article></main></body></html>').encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/html; charset=utf-8')
            self.send_header('Content-Length', str(len(page)))
            self.end_headers()
            self.wfile.write(page)
        else:
            self.send_response(503)
            self.end_headers()
            self.wfile.write(b'synthetic unavailable')

ThreadingHTTPServer(('127.0.0.1', 18080), Handler).serve_forever()
