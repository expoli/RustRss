"""Compare 20 open-and-star cycles in two native WebKitGTK desktop builds.

Both binaries receive byte-identical copies of a synthetic 200-entry database.
The clock is performance.now() inside WebKit. Open ends at the reader DOM
mutation; star ends when its row marker, reader control and sidebar count
update after real Tauri IPC. Inspector transport is outside timed intervals.
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
  if (window.__t7starperf) throw new Error('harness already started');
  window.__t7starperf = { done: false, error: null, result: null };
  (async () => {
    const list = document.getElementById('entries');
    const reader = document.getElementById('reader');
    const all = [...list.querySelectorAll('li[data-id]')];
    if (all.length !== 200) throw new Error('expected 200 rendered rows, got ' + all.length);
    const rows = all.filter(row => row.classList.contains('read') &&
      !row.querySelector('.entry-thumbnail'));
    if (rows.length < 21) throw new Error('insufficient already-read image-free rows');
    const candidates = rows.slice(0, 21);
    const originalNodes = new Map(all.map(row => [row.dataset.id, row]));
    const roots = ['entries','reader','views','feeds','tags','list-count','db-info',
      'feeds-meta','tags-meta','tags-arrow','tags-empty','m-feeds-empty']
      .map(id => document.getElementById(id)).filter(Boolean);
    const inScope = node => roots.some(root => root === node || root.contains(node));
    const writes = [], listMutations = [], sidebarMutations = [], samples = [];
    let phase = 'setup';
    const originalSetAttribute = Element.prototype.setAttribute;
    const textDescriptor = Object.getOwnPropertyDescriptor(Node.prototype, 'textContent');
    const htmlDescriptor = Object.getOwnPropertyDescriptor(Element.prototype, 'innerHTML');
    Element.prototype.setAttribute = function(name, value) {
      if (phase === 'star' && inScope(this) && this.getAttribute(name) === String(value))
        writes.push({kind:'setAttribute', node:this.tagName, id:this.id || null,
          attribute:name, sameValue:true});
      return originalSetAttribute.call(this, name, value);
    };
    Object.defineProperty(Node.prototype, 'textContent', {
      ...textDescriptor, set(value) {
        if (phase === 'star' && inScope(this) && this.textContent === String(value))
          writes.push({kind:'textContent', node:this.nodeName,
            id:this.id || this.parentElement?.id || null, sameValue:true});
        return textDescriptor.set.call(this, value);
      }
    });
    Object.defineProperty(Element.prototype, 'innerHTML', {
      ...htmlDescriptor, set(value) {
        if (phase === 'star' && inScope(this) && this.innerHTML === String(value))
          writes.push({kind:'innerHTML', node:this.tagName, id:this.id || null, sameValue:true});
        return htmlDescriptor.set.call(this, value);
      }
    });
    const observer = new MutationObserver(records => {
      for (const r of records) {
        listMutations.push({phase, type:r.type, target:r.target.nodeName,
          targetClass:r.target.className || null,
          rowId:r.target.closest?.('li[data-id]')?.dataset.id || null,
          added:[...r.addedNodes].map(n => n.nodeType === 1 ? n.tagName+'.'+n.className : n.nodeName),
          removed:[...r.removedNodes].map(n => n.nodeType === 1 ? n.tagName+'.'+n.className : n.nodeName),
          attribute:r.attributeName || null,
          sameValue:r.type === 'attributes' && r.oldValue === r.target.getAttribute(r.attributeName)});
      }
    });
    observer.observe(list, {subtree:true, childList:true, attributes:true, attributeOldValue:true});
    const sidebarObserver = new MutationObserver(records => {
      for (const r of records) {
        sidebarMutations.push({phase, type:r.type,
          target:r.target.nodeName, targetId:r.target.id || null,
          targetClass:r.target.className || null,
          owner:r.target.closest?.('li[data-kind], li[data-key]')?.dataset.kind ||
            r.target.closest?.('li[data-key]')?.dataset.key || null,
          oldValue:r.oldValue || null,
          newValue:r.type === 'attributes' ? r.target.getAttribute(r.attributeName) :
            r.type === 'characterData' ? r.target.data : r.target.textContent,
          attribute:r.attributeName || null,
          added:r.addedNodes.length, removed:r.removedNodes.length,
          sameValue:r.type === 'attributes' ? r.oldValue === r.target.getAttribute(r.attributeName) :
            r.type === 'characterData' ? r.oldValue === r.target.data : false});
      }
    });
    for (const root of roots.filter(n => n !== list && n !== reader))
      sidebarObserver.observe(root, {subtree:true, childList:true, attributes:true,
        attributeOldValue:true, characterData:true, characterDataOldValue:true});
    const open = row => new Promise((resolve, reject) => {
      const id = row.dataset.id;
      const title = row.querySelector('.title')?.textContent;
      if (!title) { reject(new Error('missing title '+id)); return; }
      const watcher = new MutationObserver(() => {
        if (reader.querySelector('.reader-head h1')?.textContent !== title ||
            !list.querySelector('li[data-id="'+id+'"]').classList.contains('active')) return;
        clearTimeout(timeout);
        watcher.disconnect();
        resolve({id, ms:+(performance.now()-started).toFixed(3),
          rowPreserved:originalNodes.get(id) === list.querySelector('li[data-id="'+id+'"]'),
          renderedRows:list.querySelectorAll('li[data-id]').length});
      });
      const timeout = setTimeout(() => {watcher.disconnect(); reject(new Error('open timeout '+id));},10000);
      watcher.observe(reader, {subtree:true, childList:true});
      phase = 'open';
      const started = performance.now();
      row.click();
    });
    const star = row => new Promise((resolve, reject) => {
      const id = row.dataset.id;
      const prior = !!list.querySelector('li[data-id="'+id+'"] .meta .star');
      const countNode = document.querySelector('#views li[data-kind="starred"] .count');
      const beforeCount = Number(countNode?.textContent);
      const button = document.getElementById('act-star');
      const beforeButton = button?.textContent;
      if (!button || !Number.isFinite(beforeCount)) {reject(new Error('star controls missing'));return;}
      const expectedCount = beforeCount + (prior ? -1 : 1);
      const finished = () => {
        const current = list.querySelector('li[data-id="'+id+'"]');
        return current && !!current.querySelector('.meta .star') !== prior &&
          Number(countNode.textContent) === expectedCount &&
          document.getElementById('act-star')?.textContent !== beforeButton;
      };
      const watcher = new MutationObserver(() => {
        if (!finished()) return;
        clearTimeout(timeout);
        watcher.disconnect();
        resolve({id, ms:+(performance.now()-started).toFixed(3), prior,
          after:!prior, starredCountBefore:beforeCount, starredCountAfter:expectedCount,
          rowPreserved:originalNodes.get(id) === list.querySelector('li[data-id="'+id+'"]'),
          renderedRows:list.querySelectorAll('li[data-id]').length});
      });
      const timeout = setTimeout(() => {watcher.disconnect(); reject(new Error('star timeout '+id));},10000);
      watcher.observe(list, {subtree:true, childList:true, attributes:true});
      watcher.observe(document.getElementById('views'), {subtree:true, childList:true, attributes:true});
      watcher.observe(reader, {subtree:true, childList:true, attributes:true});
      phase = 'star';
      const started = performance.now();
      button.click();
    });
    try {
      // Exactly one untimed open+star warmup, then 20 measured open+star cycles.
      await open(candidates[0]);
      await star(candidates[0]);
      for (const row of candidates.slice(1)) {
        const pairStart = performance.now();
        const opened = await open(row);
        const starred = await star(row);
        samples.push({id:row.dataset.id, open:opened, star:starred,
          pairMs:+(performance.now()-pairStart).toFixed(3)});
        phase = 'between';
        await new Promise(resolve => setTimeout(resolve, 25));
      }
      const current = [...list.querySelectorAll('li[data-id]')];
      const identity = current.length === 200 &&
        current.every(row => originalNodes.get(row.dataset.id) === row);
      window.__t7starperf.result = {initialRows:all.length, finalRows:current.length,
        warmupId:candidates[0].dataset.id, samples, listMutations, sidebarMutations,
        sameValueWrites:writes, identity};
    } finally {
      observer.disconnect();
      sidebarObserver.disconnect();
      Element.prototype.setAttribute = originalSetAttribute;
      Object.defineProperty(Node.prototype, 'textContent', textDescriptor);
      Object.defineProperty(Element.prototype, 'innerHTML', htmlDescriptor);
      window.__t7starperf.done = true;
    }
  })().catch(error => {window.__t7starperf.error=String(error?.stack||error);window.__t7starperf.done=true;});
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
        await wait_js(probe, "window.__t7starperf?.done", 90)
        raw = json.loads(await probe.js("JSON.stringify(window.__t7starperf)"))
        if raw["error"]:
            raise AssertionError(raw["error"])
        result = raw["result"]
        assert len(result["samples"]) == 20, len(result["samples"])
        assert result["identity"] and result["finalRows"] == 200, "list rows were rebuilt"
        child_changes = [m for m in result["listMutations"] if m["type"] == "childList"]
        assert all(m["targetClass"] == "meta" and
                   m["added"] + m["removed"] == ["SPAN.star"] for m in child_changes), \
            "unexpected list child mutation"
        assert len(child_changes) == 21, f"expected one star marker patch per cycle, got {len(child_changes)}"
        assert all(s[part]["rowPreserved"] and s[part]["renderedRows"] == 200
                   for s in result["samples"] for part in ("open", "star"))
        result["expectedStarMarkerChildMutations"] = len(child_changes)
        result["sameValueSidebarMutations"] = sum(m["sameValue"] for m in result["sidebarMutations"])
        result["sidebarOnlyStarCountMutations"] = all(
            m["phase"] == "star" and m["type"] == "childList" and
            m["owner"] == "starred" and m["targetClass"] == "count"
            for m in result["sidebarMutations"])
        result["p95Ms"] = percentile95([s["pairMs"] for s in result["samples"]])
        result["openP95Ms"] = percentile95([s["open"]["ms"] for s in result["samples"]])
        result["starP95Ms"] = percentile95([s["star"]["ms"] for s in result["samples"]])
        result["medianMs"] = sorted(s["pairMs"] for s in result["samples"])[len(result["samples"]) // 2]
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
        gates = {
            name: {
                "baselineP95Ms": baseline[key], "candidateP95Ms": candidate[key],
                "candidateMinusBaselineMs": round(candidate[key] - baseline[key], 3),
                "candidateOverBaselinePercent": round((candidate[key] / baseline[key] - 1) * 100, 2),
                "regressionBothThresholds": candidate[key] > baseline[key] * 1.2
                    and candidate[key] - baseline[key] >= 16,
            }
            for name, key in (("pair", "p95Ms"), ("open", "openP95Ms"), ("star", "starP95Ms"))
        }
        regression = any(gate["regressionBothThresholds"] for gate in gates.values())
        summary = {"baselineP95Ms": b, "candidateP95Ms": c,
                   "candidateOverBaselinePercent": round((c / b - 1) * 100, 2),
                   "candidateMinusBaselineMs": round(c - b, 3),
                   "regressionBothThresholds": regression,
                   "gates": gates,
                   "openP95Ms": {"baseline": baseline["openP95Ms"], "candidate": candidate["openP95Ms"]},
                   "starP95Ms": {"baseline": baseline["starP95Ms"], "candidate": candidate["starP95Ms"]},
                   "candidateStarSameValueWrites": len(candidate["sameValueWrites"]),
                   "candidateSameValueSidebarMutations": candidate["sameValueSidebarMutations"],
                   "candidateSidebarMutations": len(candidate["sidebarMutations"]),
                   "candidateSidebarOnlyStarCountMutations": candidate["sidebarOnlyStarCountMutations"],
                   "candidateExpectedStarMarkerChildMutations": candidate["expectedStarMarkerChildMutations"],
                   "candidateIdentityPreserved": candidate["identity"],
                   "candidateSameValueZeroWrites": candidate["zeroWrites"]["sameValues"]
                       and candidate["zeroWrites"]["nodesPreserved"]
                       and not candidate["zeroWrites"]["writes"],
                   "fixtureSha256": fixture_hash, "display": display,
                   "comparison": "one untimed warmup then 20 already-read open+star cycles; pair timing includes reader, star marker, reader control and starred sidebar count in native WebKitGTK"}
        (out / "summary.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n")
        print(json.dumps(summary, ensure_ascii=False), flush=True)
        assert not regression, "candidate crossed both regression thresholds"
        assert summary["candidateSameValueZeroWrites"], "same-value render wrote DOM"
        assert not candidate["sameValueWrites"], "star path made same-value DOM writes"
        assert candidate["sameValueSidebarMutations"] == 0, "sidebar emitted same-value mutation"
        assert candidate["sidebarOnlyStarCountMutations"], "sidebar changed beyond starred count"
    finally:
        if xvfb.poll() is None:
            xvfb.terminate()
            xvfb.wait(timeout=10)


if __name__ == "__main__":
    asyncio.run(main())
