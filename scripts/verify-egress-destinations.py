"""外呼目的地记录（T3 AC）：一次完整刷新周期（含 WebKit 侧图片流量）的全部目的地。

方法：`strace -f -e trace=connect,sendto` 跟住应用及全部子进程的 connect()。**应用日志看不到
WebKit 那一侧**，所以必须用系统调用层记录（评审 N2 点名要求）。归属用 `getaddrinfo`（A + AAAA
两族，`getent hosts` 只回第一族，会把 IPv4 的图片站误判成白名单外）。

为什么要「启动刷新 + 一条不属于该 feed 的合成条目」：
- 启动刷新（`refresh.on_start=true`）产生**订阅源**连接；
- 合成条目的缩略图指向**另一个图片主机**，而刷新不会覆盖它（feed 里没有这条），
  它的渲染产生 **WebKit 侧图片**连接。
这样一次 capture 里两类目的地齐备，且**不需要 inspector / xdotool**（strace 拖慢后 JS 往返
会超 Probe 的 10s 超时，实测踩过）。

用法：python3 scripts/verify-egress-destinations.py [证据目录]
"""
import asyncio
import importlib.util
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import re
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import threading
from datetime import datetime, timezone

sys.dont_write_bytecode = True

PUBLIC_FEED = 'https://github.blog/feed/'
PUBLIC_IMAGE = 'https://raw.githubusercontent.com/github/explore/main/topics/rust/rust.png'
RSSHUB_FEED = 'rsshub://x7/egress-fixture'  # 实例指向本地服务器（下面的 MIRROR），以证明 scheme 解析到实例主机
RSS_FIXTURE = (b"<?xml version='1.0'?><rss version='2.0'><channel><title>RSSHub Fixture</title>"
               b'<item><guid>r1</guid><title>instance resolution</title></item></channel></rss>')
RSSHUB_REQUESTS = []


class RssHubHandler(BaseHTTPRequestHandler):
    """冒充 RSSHub 实例：记录被请求的路径（用于证明 `rsshub://path` 解析到了实例主机）。"""

    def do_GET(self):
        RSSHUB_REQUESTS.append(self.path)
        self.send_response(200)
        self.send_header('Content-Type', 'application/rss+xml')
        self.send_header('Content-Length', str(len(RSS_FIXTURE)))
        self.end_headers()
        self.wfile.write(RSS_FIXTURE)

    def log_message(self, *_args):
        pass
CONNECT_RE = re.compile(
    r'connect\(\d+, \{sa_family=(AF_INET6?), sin_port=htons\((\d+)\), sin_addr=inet_addr\("([0-9a-f.:]+)"\)')
CONNECT6_RE = re.compile(
    r'connect\(\d+, \{sa_family=AF_INET6, sin6_port=htons\((\d+)\)'
    r'.*?inet_pton\(AF_INET6, "([0-9a-fA-F:]+)"')
DNS_RE = re.compile(r'sendto\(\d+, .*sin_port=htons\(53\)')
OUT_DIR = Path(sys.argv[1]) if len(sys.argv) > 1 else None


def parse_connect(line):
    """从一行 strace 里取出 (family, port, ip)；不匹配返回 None。

    IPv4 与 IPv6 都要认：只认 IPv4 会在双栈环境里**默默漏计目的地**（评审 N1）。
    """
    m = CONNECT_RE.search(line)
    if m:
        return ('AF_INET6' if m.group(1) == 'AF_INET6' else 'AF_INET', int(m.group(2)), m.group(3))
    m = CONNECT6_RE.search(line)
    if m:
        return ('AF_INET6', int(m.group(1)), m.group(2))
    return None


def write_evidence(name, payload):
    if OUT_DIR is None:
        return None
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    path = OUT_DIR / name
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + '\n')
    return str(path)


def resolve(host):
    """A + AAAA 两族都取（`getent hosts` 只回第一族，实测 IPv4 图片站会被漏掉）。"""
    try:
        infos = socket.getaddrinfo(host, 443, proto=socket.IPPROTO_TCP)
    except socket.gaierror:
        return []
    return sorted({info[4][0] for info in infos})


def stop(proc):
    """strace 包着的应用收到 SIGTERM 不会跟着退出（实测），所以按进程组 kill。"""
    if proc is None or proc.poll() is not None:
        return
    pgid = os.getpgid(proc.pid)
    if pgid == os.getpgrp():
        # 防御：子进程与自身同组（漏了 start_new_session 时）——killpg 会把自己也杀掉
        proc.terminate()
        try:
            proc.wait(timeout=8)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)
        return
    try:
        os.killpg(pgid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        proc.wait(timeout=8)
        return
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(pgid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    try:
        proc.wait(timeout=5)
    except subprocess.TimeoutExpired:
        pass


async def main():
    report = {'public_feed': PUBLIC_FEED, 'public_image': PUBLIC_IMAGE,
              'checked_at': datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
              'checks': []}
    feed_ips = resolve('github.blog')
    image_ips = resolve('raw.githubusercontent.com')
    report['feed_ips'] = feed_ips
    report['image_ips'] = image_ips

    with tempfile.TemporaryDirectory(prefix='rustrss-egress-') as temporary:
        root = Path(temporary)
        dbpath = root / 'app.sqlite'
        runtime = root / 'runtime'
        runtime.mkdir(mode=0o700)
        env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
                   XDG_RUNTIME_DIR=str(runtime), GDK_GL='disable', GDK_SCALE='1', RUSTSS_DB=str(dbpath))
        for key in ('GDK_BACKEND', 'WAYLAND_DISPLAY', 'EGL_PLATFORM'):
            env.pop(key, None)
        read_fd, write_fd = os.pipe()
        xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '1400x1000x24'],
                                pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os.close(write_fd)
        app = None
        rsshub_server = None
        try:
            with os.fdopen(read_fd) as pipe:
                env['DISPLAY'] = ':' + pipe.readline().strip()
            log_path = root / 'desktop.log'

            # 1) 先空跑一次：建基线库 → 播种真实订阅 → 让启动刷新把 feed 抓下来
            with log_path.open('a') as log:
                boot = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log,
                                        start_new_session=True)
            for _ in range(200):
                if dbpath.exists() and dbpath.stat().st_size > 0:
                    break
                await asyncio.sleep(.1)
            stop(boot)
            rsshub_server = ThreadingHTTPServer(('127.0.0.1', 0), RssHubHandler)
            threading.Thread(target=rsshub_server.serve_forever, daemon=True).start()
            mirror = f'http://127.0.0.1:{rsshub_server.server_port}'
            report['rsshub_mirror'] = mirror
            rsshub_port = rsshub_server.server_port
            with sqlite3.connect(dbpath) as db:
                db.execute('INSERT INTO feeds(url,title,created_at) VALUES(?,?,?)',
                           (PUBLIC_FEED, 'GitHub Blog', 1))
                # 第二个源用 rsshub:// 形态，实例指向本探针的本地服务器：
                # 服务器收到的路径直接证明「按当前实例解析」这一步真的发生了
                db.execute('INSERT INTO feeds(url,title,created_at) VALUES(?,?,?)',
                           (RSSHUB_FEED, 'RSSHub 实例解析源', 1))
                db.execute("INSERT INTO settings(key,value,updated_at) VALUES('rsshub.mirror',?,1) "
                           "ON CONFLICT(key) DO UPDATE SET value=excluded.value", (mirror,))
                db.execute("INSERT INTO settings(key,value,updated_at) VALUES('refresh.on_start','true',1) "
                           "ON CONFLICT(key) DO UPDATE SET value='true'")
                db.execute("INSERT INTO settings(key,value,updated_at) VALUES('refresh.interval_minutes','off',1) "
                           "ON CONFLICT(key) DO UPDATE SET value='off'")
            app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL, start_new_session=True)
            fed = 0
            for _ in range(120 * 5):
                try:
                    with sqlite3.connect(dbpath) as db:
                        fed = db.execute('SELECT COUNT(*) FROM entries').fetchone()[0]
                except sqlite3.Error:
                    fed = 0
                if fed:
                    break
                await asyncio.sleep(.2)
            assert fed > 0, '启动刷新未解析出条目（链路未走通）'
            stop(app)
            app = None
            report['feed_entries'] = fed

            # 2) 插入一条**不属于该 feed** 的合成条目：刷新不会覆盖它，它的缩略图指向另一个图片主机。
            #    时间戳给大值 → 排在列表首位 → 一定进入首屏（懒加载才会真的发请求）。
            with sqlite3.connect(dbpath) as db:
                db.execute(
                    "INSERT INTO entries(id,feed_id,stable_id,id_origin,title,url,summary,content_html,"
                    "content_text,search_tokens,content_hash,read,starred,fetched_at,published_at,thumbnail_url) "
                    "VALUES(900001,1,'synthetic-egress','source_data','外呼记录用合成条目',?, '摘要',NULL,"
                    "'正文','合成',  'h',0,0,1,2000000000,?)",
                    ('https://example.invalid/synthetic', PUBLIC_IMAGE))
            report['synthetic_entry'] = 900001

            # 3) 带 strace 启动：启动刷新（订阅源）+ 首屏渲染（图片站）在同一次 capture 里
            egress_log = (OUT_DIR / 'egress-strace.log') if OUT_DIR else Path('/tmp/egress-last.log')
            if OUT_DIR:
                OUT_DIR.mkdir(parents=True, exist_ok=True)
            if egress_log.exists():
                egress_log.unlink()
            with log_path.open('a') as log:
                app = subprocess.Popen(
                    ['strace', '-f', '-e', 'trace=connect,sendto', '-o', str(egress_log),
                     '-s', '200', 'target/debug/rustrss-desktop'],
                    env=env, stdout=log, stderr=log, start_new_session=True)
            for _ in range(120 * 5):
                if 'loaded feeds=' in log_path.read_text():
                    break
                await asyncio.sleep(.2)
            await asyncio.sleep(75)  # strace 下更慢：等启动刷新 + 首屏懒加载图片
            stop(app)
            app = None

            # 4) 解析目的地（归属按 IP：端口可能未落定，见下）
            text = egress_log.read_text(errors='replace') if egress_log.exists() else ''
            by_ip = {}
            unfinished = 0
            for line in text.splitlines():
                parsed = parse_connect(line)
                if parsed is None:
                    continue
                is_unfinished = '<unfinished' in line
                if is_unfinished:
                    unfinished += 1
                family, port, ip = parsed
                slot = by_ip.setdefault((ip, family), {'count': 0, 'ports': set()})
                slot['count'] += 1
                slot['ports'].add(port)
            report['unfinished_connect_lines'] = unfinished
            report['dns_sendto_53'] = len(DNS_RE.findall(text))

            def owner_of(ip, ports):
                if ip in feed_ips:
                    return 'feed_host'
                if ip in image_ips:
                    return 'image_host'
                # 本探针把 rsshub 实例指到本地服务器：按**端口**认出它，
                # 口径上它属于「订阅源侧」（实例主机就是订阅源的抓取出口）。
                if ip.startswith('127.') and rsshub_port in ports:
                    return 'rsshub_instance'
                if ip == '127.0.0.53' or (53 in ports and ip.startswith('127.')):
                    return 'dns'
                if ip.startswith('127.') or ip == '::1':
                    return 'loopback'  # 应用自身：MCP / loopback 服务
                return 'UNCLASSIFIED'

            entries = []
            for (ip, family), slot in sorted(by_ip.items(), key=lambda kv: -kv[1]['count']):
                ports = sorted(slot['ports'])
                entries.append({'ip': ip, 'family': family, 'count': slot['count'], 'ports': ports,
                                'port_unresolved': ports == [0],
                                'owner': owner_of(ip, slot['ports'])})
            report['destinations'] = entries
            report['raw_log'] = str(egress_log)
            report['unclassified'] = [e for e in entries if e['owner'] == 'UNCLASSIFIED']
            kinds = {e['owner'] for e in entries}
            assert 'feed_host' in kinds, f'未观察到订阅源连接: {entries}'
            assert 'image_host' in kinds, f'未观察到图片站连接（WebKit 侧）: {entries}'
            report['rsshub_instance_paths'] = list(RSSHUB_REQUESTS)
            report['rsshub_observed'] = 'rsshub_instance' in kinds
            assert report['rsshub_observed'], f'未观察到 rsshub 实例连接: {entries}'
            assert any('x7/egress-fixture' in p for p in RSSHUB_REQUESTS), \
                f'实例未收到解析后的路径（scheme 解析未发生）: {RSSHUB_REQUESTS}'
            report['checks'].append('rsshub:// resolved to the configured instance and its request path was observed')
            assert not report['unclassified'], f"白名单外目的地: {report['unclassified']}"
            report['checks'].append('complete refresh cycle: only feed host, image host, DNS and loopback')
            report['checks'].append('WebKit-side image traffic visible through strace connect()')

            # 5) 开关关闭态对照（提案评审 NOTE N3）：关掉列表缩略图后，同一次启动里
            #    不得再出现任何**图片站**连接——即「关闭开关后该类请求消失」不是口号。
            with sqlite3.connect(dbpath) as db:
                row = db.execute("SELECT value FROM settings WHERE key='ui.theme_config'").fetchone()
                theme = json.loads(row[0]) if row and row[0] else {}
                theme.setdefault('overrides', {}).setdefault('list', {})['thumbnail'] = False
                db.execute("UPDATE settings SET value=? WHERE key='ui.theme_config'",
                           (json.dumps(theme),))
                db.execute("UPDATE settings SET value='false' WHERE key='refresh.on_start'")
            off_log = (OUT_DIR / 'egress-strace-toggle-off.log') if OUT_DIR else Path('/tmp/egress-off.log')
            if off_log.exists():
                off_log.unlink()
            with log_path.open('a') as log:
                app = subprocess.Popen(
                    ['strace', '-f', '-e', 'trace=connect,sendto', '-o', str(off_log),
                     '-s', '200', 'target/debug/rustrss-desktop'],
                    env=env, stdout=log, stderr=log, start_new_session=True)
            for _ in range(120 * 5):
                if 'loaded feeds=' in log_path.read_text():
                    break
                await asyncio.sleep(.2)
            await asyncio.sleep(45)
            stop(app)
            app = None
            off_text = off_log.read_text(errors='replace') if off_log.exists() else ''
            off_dests = {}
            for line in off_text.splitlines():
                parsed = parse_connect(line)
                if parsed is None:
                    continue
                family, port, ip = parsed
                off_dests.setdefault((ip, family), {'count': 0, 'ports': set()})
                off_dests[(ip, family)]['count'] += 1
                off_dests[(ip, family)]['ports'].add(port)
            report['toggle_off_raw_log'] = str(off_log)
            report['toggle_off_destinations'] = [
                {'ip': ip, 'family': fam, 'count': slot['count'], 'ports': sorted(slot['ports']),
                 'owner': owner_of(ip, slot['ports'])}
                for (ip, fam), slot in sorted(off_dests.items(), key=lambda kv: -kv[1]['count'])]
            off_kinds = {d['owner'] for d in report['toggle_off_destinations']}
            assert 'image_host' not in off_kinds, \
                f'关闭缩略图开关后仍出现图片站连接: {report["toggle_off_destinations"]}'
            assert not [d for d in report['toggle_off_destinations'] if d['owner'] == 'UNCLASSIFIED'], \
                f"关闭态出现白名单外目的地: {report['toggle_off_destinations']}"
            report['checks'].append('thumbnail toggle off: no image-host connection at all')
            report['evidence_file'] = write_evidence('egress-destinations-results.json', report)
            print(json.dumps(report, ensure_ascii=False))
        finally:
            stop(app)
            xvfb.terminate()
            xvfb.wait(timeout=10)
            if rsshub_server is not None:
                rsshub_server.shutdown()


asyncio.run(main())
