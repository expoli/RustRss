#!/usr/bin/env python3
"""Run the isolated Tauri screenshot probe under Xvfb or native Wayland.

Requires an already rebuilt theme_snapshot example, xvfb-run, and Pillow.
No user database is copied or opened. Artifacts remain in the printed directory.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import statistics
import subprocess
import tempfile

from PIL import Image

parser = argparse.ArgumentParser()
parser.add_argument("--scale", type=int, choices=[1, 2])
parser.add_argument("--display", choices=["xvfb", "wayland"], default="xvfb")
parser.add_argument("--binary", type=Path, default=Path("target/debug/examples/theme_snapshot"))
args = parser.parse_args()
root = Path(tempfile.mkdtemp(prefix=f"rustrss-tauri-snapshot-{args.display}-"))
for name in ["home", "config", "data", "cache", "runtime"]:
    (root / name).mkdir(mode=0o700)
env = dict(os.environ, HOME=str(root / "home"), XDG_CONFIG_HOME=str(root / "config"),
           XDG_DATA_HOME=str(root / "data"), XDG_CACHE_HOME=str(root / "cache"),
           )
if args.display == "xvfb":
    env.update(XDG_RUNTIME_DIR=str(root / "runtime"), GDK_GL="disable", GDK_SCALE=str(args.scale or 1))
    prefix = ["xvfb-run", "-a", "-s", "-screen 0 2400x1800x24"]
else:
    if not env.get("WAYLAND_DISPLAY") or not env.get("XDG_RUNTIME_DIR"):
        raise SystemExit("Native Wayland requires the current session socket and runtime directory")
    env.pop("DISPLAY", None)  # Prevent XWayland fallback in this child process only.
    if args.scale:
        env["GDK_SCALE"] = str(args.scale)
    prefix = []  # Preserve the real session runtime, rendering and scale defaults.
print(f"Artifacts: {root}", flush=True)
with (root / "run.log").open("w") as log:
    result = subprocess.run(["timeout", "135s", *prefix,
                             str(args.binary.resolve()), str(root / "captures")],
                            env=env, stdout=log, stderr=subprocess.STDOUT, timeout=145)
if result.returncode:
    raise SystemExit(f"probe exited {result.returncode}; see {root / 'run.log'}")
report = json.loads((root / "captures/results.json").read_text())
expected_backend = "GdkWaylandDisplay" if args.display == "wayland" else "GdkX11Display"
assert report["display_backend"] == expected_backend, report["display_backend"]
assert "error" not in report["details"], report["details"].get("error")
frames = report["details"]["frames"]
assert len(frames) == 100
digests = []
for index, frame in enumerate(frames, 1):
    assert frame["revision"] == index
    path = root / "captures" / frame["file"]
    scale = frame["scale_factor"]
    assert frame["logical_size"] == frame["viewport"], (index, "WebView viewport mismatch")
    assert scale == frame["device_pixel_ratio"], (index, "device scale mismatch")
    with Image.open(path) as image:
        rgb = image.convert("RGB")
        assert list(image.size) == frame["pixel_size"]
        assert rgb.getpixel((round(12 * scale), round(12 * scale))) == tuple(frame["marker"]), (index, "stale marker")
        color = frame["canvas"]
        expected = tuple(int(color[i:i+2], 16) for i in (1, 3, 5))
        assert rgb.getpixel((image.width - 10, image.height - 10)) == expected, (index, "stale palette")
        assert image.width * image.height <= 6_000_000
        if args.scale is not None:
            assert scale == args.scale, frame
        assert list(image.size) == [round(n * scale) for n in frame["logical_size"]]
    assert path.stat().st_size == frame["png_bytes"] <= 2 * 1024 * 1024
    digests.append(hashlib.sha256(path.read_bytes()).hexdigest())
assert len(set(digests)) == 100
checks = report["details"]["checks"]
assert all(c["actual"] == c["expected"] for c in checks)
summary = {"platform":report["platform"], "environment":args.display,
           "display_backend":report["display_backend"],
           "rendering_override":env.get("GDK_GL"), "desktop":env.get("XDG_CURRENT_DESKTOP"),
           "scale_factor":frames[0]["scale_factor"], "frames":len(frames), "pixel_assertions":"passed",
           "unique_pngs":len(set(digests)), "pixel_size":frames[0]["pixel_size"],
           "logical_size":frames[0]["logical_size"], "device_pixel_ratio":frames[0]["device_pixel_ratio"],
           "capture_ms_median":round(statistics.median(f["capture_ms"] for f in frames), 2),
           "capture_ms_max":round(max(f["capture_ms"] for f in frames), 2),
           "max_png_bytes":max(f["png_bytes"] for f in frames), "checks":checks,
           "artifacts":str(root), "frame_sha256":digests}
(root / "verification.json").write_text(json.dumps(summary, indent=2) + "\n")
print(json.dumps({k:v for k,v in summary.items() if k != "frame_sha256"}, indent=2))
