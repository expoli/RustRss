"""Compare 20 row-click interactions in two native WebKitGTK desktop builds.

Both binaries receive byte-identical copies of a synthetic 200-entry database.
The clock is performance.now() inside WebKit, from li.click() to the reader DOM
mutation produced by the real row handler and Tauri get_entry IPC. Inspector
transport and Python polling are outside the timed interval.
"""

import argparse
import asyncio
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import shutil
import socket
import sqlite3
import subprocess
import sys
import time

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "native_probe", Path(__file__).with_name("verify-theme-native-settings.py")
)
native_probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(native_probe)


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def percentile95(values):
    return sorted(values)[math.ceil(0.95 * len(values)) - 1]


async def wait_js(probe, expression, timeout=25):
    deadline = time.monotonic() + timeout
    last = None
    while time.monotonic() < deadline:
        try:
            value = await probe.js(expression)
            if value:
                return value
        except (OSError, IndexError, ConnectionError) as error:
            last = error
        await asyncio.sleep(0.1)
    raise AssertionError(f"WebKit wait timed out: {expression}; last={last}")


HARNESS = r"""(() => {
  if (window.__t7perf) throw new Error('harness already started');
  window.__t7perf = { done: false, error: null, result: null };
  (async () => {
    const list = document.getElementById('entries');
    const reader = document.getElementById('reader');
    const all = [...list.querySelectorAll('li[data-id]')];
    if (all.length !== 200) throw new Error('expected exactly 200 rendered rows, got ' + all.length);
    const rows = all.filter(row => row.classList.contains('read') &&
      !row.querySelector('.entry-thumbnail'));
    if (rows.length < 21) throw new Error('insufficient already-read, image-free rows: ' + rows.length);
    const candidates = rows.slice(0, 21);
    const samples = [];
    const originalNodes = new Map(all.map(row => [row.dataset.id, row]));
    const listMutations = [];
    const listObserver = new MutationObserver(records => {
      for (const record of records) listMutations.push({
        type: record.type, target: record.target.nodeName,
        added: record.addedNodes.length, removed: record.removedNodes.length,
        attribute: record.attributeName || null
      });
    });
    listObserver.observe(list, {subtree: true, childList: true, attributes: true});
    const clickAndMeasure = row => new Promise((resolve, reject) => {
      const id = row.dataset.id;
      const title = row.querySelector('.title')?.textContent;
      if (!title) { reject(new Error('missing row title ' + id)); return; }
      let finished = false;
      const timeout = setTimeout(() => {
        if (!finished) { observer.disconnect(); reject(new Error('reader did not update for ' + id)); }
      }, 10000);
      const observer = new MutationObserver(() => {
        if (reader.querySelector('.reader-head h1')?.textContent !== title) return;
        if (!row.classList.contains('active')) return;
        finished = true;
        clearTimeout(timeout);
        observer.disconnect();
        resolve({id, ms: +(performance.now() - start).toFixed(3),
          readerTitle: title, rowPreserved: originalNodes.get(id) === row,
          renderedRows: list.querySelectorAll('li[data-id]').length,
          readerRenderMs: window.__RENDER_TIMINGS?.find(x => x.tag === 'total')?.ms ?? null});
      });
      observer.observe(reader, {subtree: true, childList: true});
      const start = performance.now();
      row.click();
    });
    // One untimed warmup brings code, fonts, and the SQLite page cache online.
    await clickAndMeasure(candidates[0]);
    for (const row of candidates.slice(1)) {
      samples.push(await clickAndMeasure(row));
      await new Promise(resolve => setTimeout(resolve, 25));
    }
    listObserver.disconnect();
    const current = [...list.querySelectorAll('li[data-id]')];
    const identity = current.length === 200 && current.every(row => originalNodes.get(row.dataset.id) === row);
    window.__t7perf.result = {initialRows: all.length, finalRows: current.length,
      warmupId: candidates[0].dataset.id, samples, listMutations, identity};
    window.__t7perf.done = true;
  })().catch(error => {window.__t7perf.error = String(error?.stack || error); window.__t7perf.done = true;});
  return true;
})()"""


ZERO_WRITES = r"""(() => {
  window.__t7zero = {done:false, error:null, result:null};
  (async () => {
    const ids = ['views', 'feeds', 'tags', 'list-count', 'db-info', 'feeds-meta',
      'tags-meta', 'tags-arrow', 'tags-empty', 'm-feeds-empty'];
    const nodes = ids.map(id => document.getElementById(id)).filter(Boolean);
    const counts = [...document.querySelectorAll('#views .count, #feeds .count, #tags .count'),
      document.getElementById('list-count')];
    const values = counts.map(n => n.textContent);
    const records = [];
    const observer = new MutationObserver(changes => {
      for (const r of changes) {
        const value = r.type === 'attributes' ? r.target.getAttribute(r.attributeName) :
          r.type === 'characterData' ? r.target.data : null;
        records.push({target:r.target.id || r.target.closest?.('[data-key]')?.dataset.key || r.target.nodeName,
          type:r.type, attribute:r.attributeName || null, before:r.oldValue, after:value,
          sameValue:r.type !== 'childList' && r.oldValue === value,
          added:r.addedNodes?.length || 0, removed:r.removedNodes?.length || 0});
      }
    });
    for (const node of nodes) observer.observe(node, {subtree:true, attributes:true,
      attributeOldValue:true, childList:true, characterData:true, characterDataOldValue:true});
    // Selecting the current All view runs the production sidebar/list count
    // render paths again with identical count values.
    document.querySelector('#views li[data-kind="all"]').click();
    await new Promise(resolve => setTimeout(resolve, 500));
    observer.disconnect();
    window.__t7zero.result = {observedRoots:ids, countNodes:counts.length, writes:records,
      sameValueWrites:records.filter(r => r.sameValue).length,
      sameValues:counts.every((n, i) => n.textContent === values[i]),
      nodesPreserved:counts.every(n => n.isConnected)};
    window.__t7zero.done = true;
  })().catch(error => {window.__t7zero.error=String(error?.stack||error);window.__t7zero.done=true;});
  return true;
})()"""


async def measure(label, binary, fixture, root, display, xvfb_pid):
    folder = root / label
    folder.mkdir()
    db = folder / "fixture.sqlite"
    shutil.copy2(fixture, db)
    input_fixture_hash = sha256(db)
    runtime = folder / "runtime"
    runtime.mkdir(mode=0o700)
    (folder / "home").mkdir()
    port = free_port()
    env = dict(os.environ, HOME=str(folder / "home"), XDG_DATA_HOME=str(folder / "data"),
               XDG_RUNTIME_DIR=str(runtime), DISPLAY=display, GDK_BACKEND="x11",
               GDK_GL="disable", GDK_SCALE="1", RUSTSS_DB=str(db),
               WEBKIT_INSPECTOR_HTTP_SERVER=f"127.0.0.1:{port}")
    env.pop("WAYLAND_DISPLAY", None)
    env.pop("EGL_PLATFORM", None)
    probe = native_probe.Probe({"root": str(folder), "inspector_port": port})
    log = folder / "desktop.log"
    with log.open("w") as output:
        app = subprocess.Popen([str(binary)], env=env, stdout=output, stderr=output)
    try:
        await wait_js(probe, "document.querySelectorAll('#entries li[data-id]').length > 0", 35)
        await probe.js("document.querySelector('#views li[data-kind=\"all\"]').click(); true")
        await wait_js(probe, "document.querySelectorAll('#entries li[data-id]').length === 200", 35)
        await probe.js(HARNESS)
        await wait_js(probe, "window.__t7perf?.done", 35)
        raw = json.loads(await probe.js("JSON.stringify(window.__t7perf)"))
        if raw["error"]:
            raise AssertionError(raw["error"])
        result = raw["result"]
        assert len(result["samples"]) == 20, len(result["samples"])
        assert result["identity"] and result["finalRows"] == 200, "list rows were rebuilt"
        assert not any(m["type"] == "childList" for m in result["listMutations"]), "list subtree rebuilt"
        assert all(s["rowPreserved"] and s["renderedRows"] == 200 for s in result["samples"])
        values = [s["ms"] for s in result["samples"]]
        result["p95Ms"] = percentile95(values)
        result["medianMs"] = sorted(values)[len(values) // 2]
        await probe.js(ZERO_WRITES)
        await wait_js(probe, "window.__t7zero?.done", 10)
        zero = json.loads(await probe.js("JSON.stringify(window.__t7zero)"))
        assert not zero["error"], zero["error"]
        result["zeroWrites"] = zero["result"]
        result["binarySha256"] = sha256(binary)
        result["fixtureSha256"] = input_fixture_hash
        result["fixtureAfterRunSha256"] = sha256(db)
        result["webkitInspectorPort"] = port
        result["xvfbPid"] = xvfb_pid
        (folder / "results.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
        return result
    finally:
        if app.poll() is None:
            app.terminate()
            try:
                app.wait(timeout=10)
            except subprocess.TimeoutExpired:
                app.kill()
                app.wait(timeout=10)


async def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--fixture-binary", type=Path,
                        help="required when generating rather than reusing a fixture")
    parser.add_argument("--fixture-source", type=Path,
                        help="reuse an exact previously generated synthetic fixture")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    fixture = out / "fixture-200.sqlite"
    if args.fixture_source:
        shutil.copy2(args.fixture_source.resolve(), fixture)
    else:
        assert args.fixture_binary, "provide --fixture-binary or --fixture-source"
        subprocess.run([str(args.fixture_binary.resolve()), str(fixture), "200"], check=True)
        with sqlite3.connect(fixture) as db:
            assert db.execute("SELECT COUNT(*) FROM entries").fetchone()[0] == 200
            # All displayed rows use the same offline synthetic data in both builds.
            db.execute("UPDATE entries SET thumbnail_url=NULL")
            db.executemany("INSERT INTO tags(id,name,color,pinned,sort_order,created_at) VALUES(?,?,?,?,?,0)",
                           [(1, "Performance · blue", "#4477aa", 1, 0),
                            (2, "Performance · plain", None, 0, 1)])
            db.executemany("INSERT INTO entry_tags(entry_id,tag_id) VALUES(?,?)",
                           [(entry_id, 1) for entry_id in range(1, 11)] + [(11, 2)])
            db.commit()
            db.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
    with sqlite3.connect(fixture) as db:
        assert db.execute("SELECT COUNT(*) FROM entries").fetchone()[0] == 200
        assert db.execute("SELECT COUNT(*) FROM tags").fetchone()[0] == 2
    fixture_hash = sha256(fixture)
    display_read, display_write = os.pipe()
    xvfb_binary = os.environ.get("T7_XVFB") or shutil.which("Xvfb") or "/tmp/rustrss-t4-xvfb/root/usr/bin/Xvfb"
    xvfb = subprocess.Popen([xvfb_binary, "-displayfd", str(display_write), "-screen", "0", "1400x1000x24", "-nolisten", "tcp"],
                            pass_fds=(display_write,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.close(display_write)
    try:
        with os.fdopen(display_read) as stream:
            display = ":" + stream.readline().strip()
        assert display != ":" and xvfb.poll() is None, "private Xvfb failed"
        baseline = await measure("baseline", args.baseline.resolve(), fixture, out, display, xvfb.pid)
        candidate = await measure("candidate", args.candidate.resolve(), fixture, out, display, xvfb.pid)
        assert baseline["fixtureSha256"] == candidate["fixtureSha256"] == fixture_hash
        b, c = baseline["p95Ms"], candidate["p95Ms"]
        regression = c > b * 1.2 and c - b > 16
        summary = {"baselineP95Ms": b, "candidateP95Ms": c,
                   "candidateOverBaselinePercent": round((c / b - 1) * 100, 2),
                   "candidateMinusBaselineMs": round(c - b, 3),
                   "regressionBothThresholds": regression,
                   "candidateIdentityPreserved": candidate["identity"],
                   "candidateSameValueZeroWrites": candidate["zeroWrites"]["sameValues"]
                       and candidate["zeroWrites"]["nodesPreserved"]
                       and not candidate["zeroWrites"]["writes"],
                   "fixtureSha256": fixture_hash, "display": display,
                   "comparison": "20 warmed already-read row clicks; click to reader DOM mutation in native WebKitGTK"}
        (out / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n")
        print(json.dumps(summary, ensure_ascii=False), flush=True)
        assert not regression, "candidate crossed both regression thresholds"
        assert summary["candidateSameValueZeroWrites"], "same-value render wrote DOM"
    finally:
        if xvfb.poll() is None:
            xvfb.terminate()
            xvfb.wait(timeout=10)


if __name__ == "__main__":
    asyncio.run(main())
