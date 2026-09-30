"""Native Android M15 refresh and notification probe on owned emulator-5586.

All feeds and the RSSHub mirror point to a local host fixture through 10.0.2.2.
Requires a clean task-owned AVD, installed debug APK, system Python websockets,
and the checked-in synthetic performance SQLite fixture as a schema template.
"""

import argparse
import asyncio
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shlex
import shutil
import sqlite3
import subprocess
import threading
import time
import urllib.request

import websockets

SERIAL = "emulator-5586"
PKG = "tech.expoli.rustrss"
ADB = "/usr/lib/android-sdk/platform-tools/adb"
CDP_PORT = 9246
LOCK = threading.Lock()
HITS = []
TRAFFIC = {"phase": "setup", "active": 0, "maximum": 0, "notify_version": 0}


def adb(*args, timeout=30, binary=False):
    result = subprocess.run([ADB, "-s", SERIAL, *map(str, args)], capture_output=True,
                            timeout=timeout)
    if result.returncode:
        raise subprocess.CalledProcessError(result.returncode, result.args,
                                            output=result.stdout, stderr=result.stderr)
    return result.stdout if binary else result.stdout.decode(errors="replace").strip()


def adb_sql(sql):
    return adb("shell", f"run-as {PKG} /system/bin/sqlite3 rustrss/rustrss.sqlite {shlex.quote(sql)}")


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


class FixtureHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def do_GET(self):
        with LOCK:
            TRAFFIC["active"] += 1
            TRAFFIC["maximum"] = max(TRAFFIC["maximum"], TRAFFIC["active"])
            hit = {"path": self.path, "phase": TRAFFIC["phase"],
                   "time": time.time(), "monotonic": time.monotonic()}
            HITS.append(hit)
            version = TRAFFIC["notify_version"]
        try:
            if self.path.startswith("/rsshub") or self.path in ("/", "/version", "/feed/rsshub/rss"):
                status, body = 503, b"local synthetic RSSHub 503"
            else:
                if self.path.startswith("/concurrency/"):
                    time.sleep(0.8)
                item = ""
                if self.path.startswith("/notify") and version:
                    item = ("<item><guid>m15-notify-%d</guid><title>M15 new article %d</title>"
                            "<description>Local notification fixture</description></item>" % (version, version))
                body = (f"<rss version='2.0'><channel><title>M15 {self.path}</title>{item}</channel></rss>").encode()
                status = 200
            self.send_response(status)
            self.send_header("Content-Type", "application/rss+xml; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            with LOCK:
                hit["status"] = status
        except (BrokenPipeError, ConnectionResetError):
            with LOCK:
                hit["disconnected"] = True
        finally:
            with LOCK:
                TRAFFIC["active"] -= 1

    def log_message(self, *_):
        pass


def phase(name):
    with LOCK:
        TRAFFIC["phase"] = name
        TRAFFIC["maximum"] = 0


def hits(name):
    with LOCK:
        return [dict(hit) for hit in HITS if hit["phase"] == name]


def active_max():
    with LOCK:
        return TRAFFIC["maximum"]


def make_fixture(template, target, port):
    shutil.copy2(template, target)
    now = int(time.time())
    with sqlite3.connect(target) as db:
        db.execute("PRAGMA foreign_keys=ON")
        db.execute("DELETE FROM entry_tags")
        db.execute("DELETE FROM entries")
        db.execute("DELETE FROM feeds")
        db.execute("DELETE FROM tags")
        db.execute("DELETE FROM folders")
        db.executemany("INSERT INTO feeds(id,url,title,last_fetched_at,created_at,refresh_interval_minutes) VALUES(?,?,?,?,?,?)",
                       [(1, f"http://10.0.2.2:{port}/interval/global", "Global 15", now - 1800, now, None),
                        (2, f"http://10.0.2.2:{port}/interval/override", "Override 120", now - 1800, now, 120)])
        for key, value in [("refresh.interval_minutes", "off"), ("refresh.on_start", "false"),
                           ("refresh.concurrency", "6"), ("notify.new_articles", "false"),
                           ("ui.locale", "en")]:
            db.execute("INSERT INTO settings(key,value,updated_at) VALUES(?,?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at", (key, value, now))
        db.commit()
        db.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()


def install_fixture(path):
    adb("shell", "am", "force-stop", PKG)
    adb("push", str(path), "/data/local/tmp/rustrss-t7-m15.sqlite", timeout=60)
    adb("shell", "run-as", PKG, "mkdir", "-p", "rustrss")
    for sidecar in ("rustrss.sqlite-wal", "rustrss.sqlite-shm"):
        try:
            adb("shell", "run-as", PKG, "rm", f"rustrss/{sidecar}")
        except subprocess.CalledProcessError:
            pass
    adb("shell", "run-as", PKG, "cp", "/data/local/tmp/rustrss-t7-m15.sqlite", "rustrss/rustrss.sqlite")


def start_app():
    start = time.monotonic()
    adb("shell", "am", "start", "-n", f"{PKG}/.MainActivity")
    deadline = start + 20
    pid = ""
    while time.monotonic() < deadline:
        try:
            pid = adb("shell", "pidof", PKG)
        except subprocess.CalledProcessError:
            pid = ""
        if pid:
            break
        time.sleep(0.1)
    assert pid, "Android process did not start"
    adb("forward", f"tcp:{CDP_PORT}", f"localabstract:webview_devtools_remote_{pid}")
    return {"pid": int(pid.split()[0]), "startMonotonic": start, "startWall": time.time()}


def target():
    with urllib.request.urlopen(f"http://127.0.0.1:{CDP_PORT}/json", timeout=3) as response:
        rows = json.load(response)
    return next(row for row in rows if row.get("type") == "page" and json.loads(row.get("description") or "{}").get("attached"))


async def js(expression):
    page = target()
    async with websockets.connect(page["webSocketDebuggerUrl"], open_timeout=4) as socket:
        await socket.send(json.dumps({"id": 1, "method": "Runtime.evaluate", "params": {
            "expression": expression, "returnByValue": True, "awaitPromise": True}}))
        while True:
            answer = json.loads(await asyncio.wait_for(socket.recv(), 15))
            if answer.get("id") != 1:
                continue
            value = answer["result"]
            assert not value.get("exceptionDetails"), value["exceptionDetails"]
            return value["result"].get("value")


async def until(expression, seconds=20):
    deadline = time.monotonic() + seconds
    last = None
    while time.monotonic() < deadline:
        try:
            last = await js(expression)
            if last:
                return last
        except Exception as error:
            last = str(error)
        await asyncio.sleep(0.15)
    raise AssertionError(f"Timed out: {expression}; last={last}")


async def ready():
    await until("document.readyState==='complete' && !!window.__TAURI__ && !!document.querySelector('#btn-refresh')", 35)
    await js("window.__m15Taps=[];document.addEventListener('click',e=>{const n=e.target.closest('button');if(n)window.__m15Taps.push({id:n.id,trusted:e.isTrusted})},true);true")


async def native_tap(selector):
    pos = await js("(() => {const n=document.querySelector(" + json.dumps(selector) + ");"
                   "if(!n)return null;n.scrollIntoView({block:'center'});const r=n.getBoundingClientRect();"
                   "return {x:r.x+r.width/2,y:r.y+r.height/2,w:r.width,h:r.height,dpr:devicePixelRatio,"
                   "visible:!!n.getClientRects().length&&document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)?.closest("
                   + json.dumps(selector) + ")===n}})()")
    assert pos and pos["visible"], f"native tap target not visible: {selector} {pos}"
    description = json.loads(target()["description"])
    x = round(description["screenX"] + pos["x"] * pos["dpr"])
    y = round(description["screenY"] + pos["y"] * pos["dpr"])
    adb("shell", "input", "tap", x, y)
    return {"selector": selector, "x": x, "y": y, **pos}


def screenshot(out, name):
    path = out / f"{name}.png"
    path.write_bytes(adb("exec-out", "screencap", "-p", binary=True, timeout=40))
    return str(path.name)


def dump_native(out, name):
    (out / f"{name}-notification.txt").write_text(adb("shell", "dumpsys", "notification", "--noredact") + "\n")
    (out / f"{name}-logcat.txt").write_text(adb("logcat", "-d", "-v", "epoch", "-t", "5000") + "\n")
    log_names = adb("shell", "run-as", PKG, "ls", "rustrss/logs").splitlines()
    if log_names:
        name_latest = sorted(log_names)[-1]
        (out / f"{name}-app.log").write_text(adb("shell", "run-as", PKG, "cat", f"rustrss/logs/{name_latest}") + "\n")


def update_settings(**values):
    now = int(time.time())
    statements = ["BEGIN"]
    for key, value in values.items():
        key = key.replace("'", "''")
        value = str(value).replace("'", "''")
        statements.append(f"INSERT INTO settings(key,value,updated_at) VALUES('{key}','{value}',{now}) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at")
    statements.append("COMMIT")
    adb_sql(";".join(statements) + ";")


def replace_feeds(template, target, port, kind, count=1, **settings):
    make_fixture(template, target, port)
    now = int(time.time())
    with sqlite3.connect(target) as db:
        db.execute("PRAGMA foreign_keys=ON")
        db.execute("DELETE FROM entries")
        db.execute("DELETE FROM feeds")
        db.executemany("INSERT INTO feeds(id,url,title,last_fetched_at,created_at) VALUES(?,?,?,?,?)",
                       [(index,
                         f"http://10.0.2.2:{port}/{kind}/{index}" if kind != "notify" else f"http://10.0.2.2:{port}/notify",
                         f"M15 {kind} {index}", now, now)
                        for index in range(1, count + 1)])
        for key, value in settings.items():
            db.execute("INSERT INTO settings(key,value,updated_at) VALUES(?,?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at",
                       (key, str(value), now))
        db.commit()
        db.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
    install_fixture(target)
    return sha256(target)


async def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--apk", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    assert os.environ.get("ANDROID_SERIAL") == SERIAL, "ANDROID_SERIAL must be emulator-5586"
    assert adb("emu", "avd", "name").splitlines()[0].strip() == "RustRssT7M15"
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    report = {"device": SERIAL, "avd": "RustRssT7M15", "apkSha256": sha256(args.apk),
              "android": adb("shell", "getprop", "ro.build.version.release"), "checks": [],
              "phases": {}, "passed": False}
    assert report["apkSha256"] == "0e011c2fa340c0a8c33b90e5d83a89696d35cd78705350fdca14a9b585043091"
    server = ThreadingHTTPServer(("127.0.0.1", 0), FixtureHandler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    port = server.server_port
    fixture = out / "fixture.sqlite"
    template = Path(__file__).resolve().parents[1] / ".chorus/specs/rss-reader/2026-09-30-reading-experience/evidence/t7-final/performance/fixture-200.sqlite"
    make_fixture(template, fixture, port)
    report["fixtureSha256"] = sha256(fixture)
    report["serverPort"] = port
    def check(name, ok, **details):
        assert ok, f"{name}: {details}"
        report["checks"].append({"name": name, **details})
        print("PASS", name, details, flush=True)
    try:
        install_fixture(fixture)
        phase("interval-initial-resume")
        report["phases"]["intervalStart"] = start_app()
        await ready()
        await asyncio.sleep(2)
        report["phases"]["intervalInitialResumeHits"] = hits("interval-initial-resume")
        # Re-age the two feeds in the live isolated SQLite after initial resume,
        # then enable the 15-minute global interval. The 120-minute override stays fresh.
        now = int(time.time())
        adb_sql(f"UPDATE feeds SET last_fetched_at={now-1800};")
        update_settings(**{"refresh.interval_minutes": "15"})
        phase("interval-tick")
        tick_started = time.monotonic()
        for _ in range(90):
            if hits("interval-tick"):
                break
            await asyncio.sleep(1)
        interval_hits = hits("interval-tick")
        report["phases"]["interval"] = {"hits": interval_hits, "waitSeconds": time.monotonic()-tick_started,
                                         "dbRows": adb_sql("SELECT id,last_fetched_at,refresh_interval_minutes FROM feeds ORDER BY id;")}
        check("global-15-override-120-tick", [h["path"] for h in interval_hits] == ["/interval/global"],
              elapsed=round(report["phases"]["interval"]["waitSeconds"], 2))
        screenshot(out, "interval-after-tick")
        dump_native(out, "interval")

        # Thirteen delayed local feeds; UI state is reloaded from the configured
        # concurrency setting, then a physical Android tap hits Refresh all.
        report["phases"]["concurrencyFixtureSha256"] = replace_feeds(
            template, out / "concurrency-fixture.sqlite", port, "concurrency", 13,
            **{"refresh.interval_minutes": "off", "refresh.on_start": "false", "refresh.concurrency": "12"})
        phase("concurrency-prime")
        start_app()
        await ready()
        await asyncio.sleep(2)
        report["phases"]["concurrencyPrimeHits"] = hits("concurrency-prime")
        check("ui-concurrency-12-loaded", (await js("window.__TAURI__.core.invoke('get_ui_settings')"))["refresh_concurrency"] == 12)
        await until("!document.querySelector('#btn-refresh').disabled")
        phase("concurrency-manual")
        tap = await native_tap("#btn-refresh")
        await until("document.querySelector('#btn-refresh').disabled", 5)
        await until("!document.querySelector('#btn-refresh').disabled", 30)
        await asyncio.sleep(0.3)
        manual_hits = hits("concurrency-manual")
        max_active = active_max()
        report["phases"]["concurrency"] = {"hits": manual_hits, "maxActive": max_active,
                                           "tap": tap, "trustedTaps": await js("window.__m15Taps")}
        check("ui-refresh-all-13-feeds-concurrency-12", len(manual_hits) == 13 and
              max_active == 12 and any(t["id"] == "btn-refresh" and t["trusted"] for t in report["phases"]["concurrency"]["trustedTaps"]),
              requests=len(manual_hits), maxActive=max_active)
        screenshot(out, "concurrency-after-refresh")
        dump_native(out, "concurrency")

        # Cold process starts only; periodic scheduling disabled. Any request
        # before 10 seconds is tracked separately as Android onResume.
        for enabled in (False, True):
            name = f"startup-{str(enabled).lower()}"
            report["phases"][name+"FixtureSha256"] = replace_feeds(
                template, out / f"{name}-fixture.sqlite", port, "startup",
                **{"refresh.on_start": str(enabled).lower(), "refresh.interval_minutes": "off"})
            phase(name)
            started = start_app()
            await ready()
            await asyncio.sleep(13)
            observed = hits(name)
            for h in observed:
                h["sinceStartSeconds"] = round(h["monotonic"]-started["startMonotonic"], 3)
            report["phases"][name] = {"start": started, "hits": observed}
            screenshot(out, name)
            dump_native(out, name)
            late = [h for h in observed if h["sinceStartSeconds"] >= 9]
            check(f"cold-start-{str(enabled).lower()}-10s", bool(late) == enabled,
                  hits=[(h["path"],h["sinceStartSeconds"]) for h in observed])

        # Background-refresh path via Android onResume, then manual negative
        # control. Both use a local feed; notification permission is read back.
        report["phases"]["notifyFixtureSha256"] = replace_feeds(
            template, out / "notify-fixture.sqlite", port, "notify",
            **{"refresh.on_start": "false", "notify.new_articles": "true", "refresh.interval_minutes": "off"})
        phase("notify-prime")
        start_app()
        await ready()
        await asyncio.sleep(2)
        report["phases"]["notifyPrimeHits"] = hits("notify-prime")
        permission = adb("shell", "dumpsys", "package", PKG)
        check("android-notification-permission", "android.permission.POST_NOTIFICATIONS: granted=true" in permission)
        phase("notify-background")
        with LOCK:
            TRAFFIC["notify_version"] = 1
        adb("shell", "input", "keyevent", "KEYCODE_HOME")
        await asyncio.sleep(0.7)
        adb("shell", "am", "start", "-n", f"{PKG}/.MainActivity")
        for _ in range(120):
            if hits("notify-background"):
                break
            await asyncio.sleep(0.1)
        await asyncio.sleep(2)
        unread = int(adb_sql("SELECT COUNT(*) FROM entries WHERE read=0;"))
        notification_before = adb("shell", "dumpsys", "notification", "--noredact")
        active_records_before = [line for line in notification_before.splitlines() if "NotificationRecord(" in line and f"pkg={PKG}" in line]
        report["phases"]["notifyBackground"] = {"hits": hits("notify-background"), "unread": unread,
                                                  "activeRecords": active_records_before}
        check("background-positive-delta-notification", unread == 1 and bool(active_records_before) and
              len(hits("notify-background")) >= 1, unread=unread, records=len(active_records_before))
        screenshot(out, "notification-background")
        dump_native(out, "notification-background")

        phase("notify-manual")
        with LOCK:
            TRAFFIC["notify_version"] = 2
        await ready()
        await until("!document.querySelector('#btn-refresh').disabled")
        await native_tap("#btn-refresh")
        await until("document.querySelector('#btn-refresh').disabled", 5)
        await until("!document.querySelector('#btn-refresh').disabled", 20)
        await asyncio.sleep(1)
        unread_after = int(adb_sql("SELECT COUNT(*) FROM entries WHERE read=0;"))
        notification_after = adb("shell", "dumpsys", "notification", "--noredact")
        active_records_after = [line for line in notification_after.splitlines() if "NotificationRecord(" in line and f"pkg={PKG}" in line]
        report["phases"]["notifyManual"] = {"hits": hits("notify-manual"), "unread": unread_after,
                                              "activeRecords": active_records_after}
        check("manual-refresh-no-extra-notification", unread_after == 2 and
              len(active_records_after) == len(active_records_before), unread=unread_after,
              notificationsBefore=len(active_records_before), notificationsAfter=len(active_records_after))
        screenshot(out, "notification-manual-control")
        dump_native(out, "notification-manual")

        # Local RSSHub endpoint responds 503 on all four probe paths.
        phase("rsshub-503")
        await js("document.querySelector('#btn-settings').click();true")
        await until("!!document.querySelector('#tab-subscriptions')?.getClientRects().length")
        await js("document.querySelector('#tab-subscriptions').click();true")
        await until("!!document.querySelector('#set-rsshub-mirror')?.getClientRects().length")
        await js("(() => {const n=document.querySelector('#set-rsshub-mirror');n.value=" +
                 json.dumps(f"http://10.0.2.2:{port}") + ";n.dispatchEvent(new Event('input',{bubbles:true}));return true})()")
        rsshub_tap = await native_tap("#btn-rsshub-test")
        await until("document.querySelector('#rsshub-status').textContent.includes('2xx')", 25)
        rsshub_status = await js("document.querySelector('#rsshub-status').textContent")
        rsshub_hits = hits("rsshub-503")
        report["phases"]["rsshub"] = {"tap": rsshub_tap, "hits": rsshub_hits, "uiStatus": rsshub_status}
        check("rsshub-all-four-local-503", [h["path"] for h in rsshub_hits] ==
              ["/version", "/", "/rsshub/rss", "/feed/rsshub/rss"] and
              all(h.get("status") == 503 for h in rsshub_hits), status=rsshub_status)
        screenshot(out, "rsshub-503")
        dump_native(out, "rsshub")
        report["passed"] = True
    finally:
        with LOCK:
            report["allHits"] = list(HITS)
        (out / "results.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
        server.shutdown()
        server.server_close()
        try:
            adb("forward", "--remove", f"tcp:{CDP_PORT}")
        except subprocess.CalledProcessError:
            pass
    print(json.dumps({"passed": report["passed"], "checks": len(report["checks"]),
                      "fixtureSha256": report["fixtureSha256"]}), flush=True)


if __name__ == "__main__":
    asyncio.run(main())
