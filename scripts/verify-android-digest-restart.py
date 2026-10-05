#!/usr/bin/env python3
"""Offline stale-slot fixture on an isolated debug emulator, then real native startup.

Do not run on a user device. Native processes are force-stopped before copying any
SQLite file; all sidecars are captured, checkpointed locally, then removed BEFORE
replacing the main file. This tests startup recovery, not live AI cancellation.
"""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import time

serial = os.environ['ANDROID_SERIAL']
assert serial.startswith('emulator-'), 'Only isolated emulators are supported'
assert os.environ.get('DIGEST_ISOLATED_FIXTURE') == '1', 'Acknowledge isolated disposable app data'
adb = os.environ.get('ADB', '/usr/lib/android-sdk/platform-tools/adb')
pkg = 'tech.expoli.rustrss'
remote = 'rustrss/rustrss.sqlite'
out = Path(sys.argv[1] if len(sys.argv) > 1 else '/tmp/rustrss-digest-restart')
out.mkdir(parents=True, exist_ok=True)


def run(*args):
    return subprocess.check_output([adb, '-s', serial, *args], timeout=30)


def stop():
    run('shell', 'am', 'force-stop', pkg)


def snapshot(folder):
    dest = folder / 'rustrss.sqlite'
    for suffix in ['', '-wal', '-shm']:
        try:
            data = run('exec-out', 'run-as', pkg, 'cat', remote + suffix)
        except subprocess.CalledProcessError:
            assert suffix, 'Main database must exist'
            continue
        Path(str(dest) + suffix).write_bytes(data)
    assert dest.read_bytes().startswith(b'SQLite format 3'), 'Expected the initialized app database'
    return dest


def install_db(dest):
    run('push', str(dest), '/data/local/tmp/rustrss-digest-restart.sqlite')
    # Explicit offline sidecar-first replacement in disposable fixture only.
    run('shell', 'run-as', pkg, 'rm', '-f', remote + '-wal', remote + '-shm')
    run('shell', 'run-as', pkg, 'cp', '/data/local/tmp/rustrss-digest-restart.sqlite', remote)


stop()
with tempfile.TemporaryDirectory(prefix='rustrss-digest-restart-') as temp:
    root = Path(temp)
    before = snapshot(root)
    with sqlite3.connect(before) as db:
        db.execute('DELETE FROM digests WHERE report_day=?', ('2026-10-04',))
        db.execute("""INSERT INTO digests(report_day, timezone_label, day_start_at, day_end_at,
            utc_offset_start,utc_offset_end,scope_key,scope_json,profile_key,profile_json,
            revision,generated_at,checkpoint_at,manifest_hash,article_count,created_at,active_job_id)
            VALUES('2026-10-04','fixture',1791072000,1791158400,0,0,'all','{}','restart-fixture','{}',
            1,20,10,'saved',0,10,'killed-fixture-job')""")
        slot = db.execute('SELECT last_insert_rowid()').fetchone()[0]
        db.execute("INSERT INTO digest_bodies(digest_id,revision,schema_ver,content_json,markdown) VALUES(?,1,1,'{}','# saved restart report')", (slot,))
    db.close()  # last connection closes and checkpoints the captured WAL
    install_db(before)
    run('shell', 'monkey', '-p', pkg, '-c', 'android.intent.category.LAUNCHER', '1')
    time.sleep(8)
    logs = run('shell', 'run-as', pkg, 'find', 'rustrss/logs', '-type', 'f').decode().splitlines()
    log = run('exec-out', 'run-as', pkg, 'cat', sorted(logs)[-1]).decode(errors='replace')
    stop()
    after_dir = root / 'after'
    after_dir.mkdir()
    after = snapshot(after_dir)
    with sqlite3.connect(after) as db:
        active, body = db.execute('SELECT d.active_job_id,b.markdown FROM digests d JOIN digest_bodies b ON b.digest_id=d.id WHERE d.id=?', (slot,)).fetchone()
    assert active is None, 'Native startup must clear the residual marker'
    assert body == '# saved restart report', 'Native startup must preserve the previous report'
    assert '已清理 1 个中断日报生成标记' in log, 'Latest native startup log must show recovery'
    result = {
        'serial': serial, 'startupMarker': active, 'preservedMarkdown': body,
        'nativeRecoveryLogObserved': True,
        'recoveryLog': [line for line in log.splitlines() if '中断日报生成标记' in line],
        'scope': 'Actual AppState startup on Android, offline residual-marker fixture; no real AI or vendor ROM kill.',
        'snapshotSha256': hashlib.sha256(after.read_bytes()).hexdigest(),
    }
    (out / 'restart-result.json').write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps(result, ensure_ascii=False, indent=2))
