"""Run the existing real-component matrix inside a private virtual KWin session.

Invoke with: dbus-run-session -- /usr/bin/python3 scripts/verify-reading-experience-virtual.py EVIDENCE_DIR
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

evidence = Path(sys.argv[1]).resolve()
evidence.mkdir(parents=True, exist_ok=True)
root = Path(tempfile.mkdtemp(prefix="rustrss-reading-kwin-"))
runtime = root / "runtime"
runtime.mkdir(mode=0o700)
env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime))
env.pop("DISPLAY", None)
env.pop("WAYLAND_DISPLAY", None)
socket_name = "wayland-reading-t1"
with (root / "kwin.log").open("w") as log:
    kwin = subprocess.Popen(
        ["kwin_wayland", "--virtual", "--socket", socket_name, "--width", "1600", "--height", "1000", "--no-lockscreen", "--no-global-shortcuts"],
        env=env, stdout=log, stderr=subprocess.STDOUT,
    )
try:
    deadline = time.monotonic() + 20
    while not (runtime / socket_name).exists():
        if kwin.poll() is not None or time.monotonic() > deadline:
            raise RuntimeError(f"virtual KWin did not start; see {root / 'kwin.log'}")
        time.sleep(0.1)
    env["WAYLAND_DISPLAY"] = socket_name
    probe = subprocess.run(
        ["/usr/bin/python3", "scripts/verify-theme-ui.py", "--display", "wayland", "--evidence", str(evidence / "theme-ui.json")],
        env=env, capture_output=True, text=True, timeout=180,
    )
    (evidence / "probe.log").write_text(probe.stdout + probe.stderr)
    if probe.returncode:
        raise RuntimeError(f"theme UI probe failed ({probe.returncode}); see {evidence / 'probe.log'}")
    result = json.loads(probe.stdout.splitlines()[-1])
    captures = Path(result["output"]) / "captures"
    report = json.loads((evidence / "theme-ui.json").read_text())
    for shot in report["captures"]:
        shutil.copy2(captures / shot["file"], evidence / ("theme-" + shot["file"]))
    settings_dir = root / "settings-captures"
    settings = subprocess.run(
        ["target/debug/examples/theme_ui", str(settings_dir), "--settings"],
        env=env, capture_output=True, text=True, timeout=120,
    )
    (evidence / "settings.log").write_text(settings.stdout + settings.stderr)
    if settings.returncode:
        raise RuntimeError(f"theme settings probe failed ({settings.returncode}); see {evidence / 'settings.log'}")
    settings_report = json.loads((settings_dir / "results.json").read_text())
    if settings_report.get("error"):
        raise RuntimeError(f"theme settings probe failed: {settings_report['error']}")
    shutil.copy2(settings_dir / "results.json", evidence / "theme-settings.json")
    for shot in settings_dir.glob("*.png"):
        shutil.copy2(shot, evidence / ("settings-" + shot.name))
    preview = subprocess.run(
        ["/usr/bin/python3", "scripts/verify-theme-preview.py", "--display", "wayland"],
        env=env, capture_output=True, text=True, timeout=180,
    )
    (evidence / "preview.log").write_text(preview.stdout + preview.stderr)
    if preview.returncode:
        raise RuntimeError(f"theme preview probe failed ({preview.returncode}); see {evidence / 'preview.log'}")
    preview_root = Path(next(line.removeprefix("Evidence:").strip() for line in preview.stdout.splitlines() if line.startswith("Evidence:")))
    shutil.copy2(preview_root / "results.json", evidence / "theme-preview.json")
    for shot in preview_root.glob("*.png"):
        shutil.copy2(shot, evidence / ("preview-" + shot.name))
    shutil.copy2(root / "kwin.log", evidence / "kwin.log")
    print(json.dumps({"captures": len(report["captures"]), "checks": len(report["checks"]), "distinct_scene_frames": report["scene_frames_per_cell"], "settings_checks": len(settings_report["checks"]), "preview": "passed", "evidence": str(evidence)}))
finally:
    kwin.terminate()
    try:
        kwin.wait(timeout=10)
    except subprocess.TimeoutExpired:
        kwin.kill()
        kwin.wait(timeout=10)
