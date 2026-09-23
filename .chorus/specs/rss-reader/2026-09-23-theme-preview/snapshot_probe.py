#!/usr/bin/env python3
"""Development probe: native WebKitGTK snapshots, not product/Tauri integration.

Run with system Python under an existing display or xvfb-run. Requires GI,
WebKit2 4.1, libcairo and Pillow. Uses a local mock; never opens the user database.
"""
import argparse
import ctypes
import ctypes.util
import hashlib
import json
import time
from pathlib import Path

import gi

gi.require_version("Gtk", "3.0")
gi.require_version("WebKit2", "4.1")
from gi.repository import GLib, Gtk, WebKit2
from PIL import Image

parser = argparse.ArgumentParser()
parser.add_argument("html", type=Path)
parser.add_argument("output", type=Path)
args = parser.parse_args()
args.output.mkdir(parents=True, exist_ok=True)
manager = WebKit2.UserContentManager()
manager.register_script_message_handler("ready")
view = WebKit2.WebView(user_content_manager=manager)
# Use the native finish/PNG functions: this host lacks gi._gi_cairo. No system
# packages are changed. The product implementation will use Rust bindings.
native = ctypes.CDLL(ctypes.util.find_library("webkit2gtk-4.1"))
cairo = ctypes.CDLL(ctypes.util.find_library("cairo"))
capsule_pointer = ctypes.pythonapi.PyCapsule_GetPointer
capsule_pointer.argtypes = [ctypes.py_object, ctypes.c_char_p]
capsule_pointer.restype = ctypes.c_void_p
native.webkit_web_view_get_snapshot_finish.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
native.webkit_web_view_get_snapshot_finish.restype = ctypes.c_void_p
cairo.cairo_surface_write_to_png.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
cairo.cairo_surface_write_to_png.restype = ctypes.c_int
cairo.cairo_surface_destroy.argtypes = [ctypes.c_void_p]
window = Gtk.Window(title="RustRss native snapshot probe")
window.set_default_size(1280, 900)
window.add(view)
states = [("clear", "light", "#ff0000"), ("paper", "dark", "#00ff00"),
          ("slate", "light", "#0000ff")] * 2
records = []
failure = []
started = time.monotonic()


def fail(error):
    failure.append(str(error))
    Gtk.main_quit()
    return False


def apply_state():
    global started
    started = time.monotonic()
    palette, mode, marker = states[len(records)]
    script = """(() => {
      const [palette, mode, marker, revision] = %s;
      theme(palette);
      if (document.documentElement.dataset.mode !== mode) document.getElementById('mode').click();
      let tag = document.getElementById('probe-marker');
      if (!tag) { tag = document.createElement('div'); tag.id='probe-marker'; document.body.append(tag); }
      tag.style.cssText='position:fixed;left:0;top:0;width:24px;height:24px;z-index:999999;background:'+marker;
      document.fonts.ready.then(() => requestAnimationFrame(() => requestAnimationFrame(() => {
        window.webkit.messageHandlers.ready.postMessage(JSON.stringify({revision, palette, mode}));
      })));
    })()""" % json.dumps([palette, mode, marker, len(records) + 1])
    view.evaluate_javascript(script, -1, None, None, None, None, None)


def captured(webview, result, metadata):
    try:
        surface = native.webkit_web_view_get_snapshot_finish(
            capsule_pointer(webview.__gpointer__, None),
            capsule_pointer(result.__gpointer__, None), None)
        if not surface:
            raise RuntimeError("native snapshot finish failed")
        path = args.output / (str(metadata["revision"]) + ".png")
        try:
            if cairo.cairo_surface_write_to_png(surface, str(path).encode()) != 0:
                raise RuntimeError("native PNG encoding failed")
        finally:
            cairo.cairo_surface_destroy(surface)
        with Image.open(path) as img:
            rgb = img.convert("RGB")
            expected = [(255, 0, 0), (0, 255, 0), (0, 0, 255)][len(records) % 3]
            if rgb.getpixel((12, 12)) != expected:
                raise RuntimeError("snapshot has stale or missing revision marker")
            if len(rgb.getcolors(rgb.width * rgb.height)) < 100:
                raise RuntimeError("snapshot content is unexpectedly blank")
            size = list(img.size)
        records.append({**metadata, "size": size, "png_bytes": path.stat().st_size,
                        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                        "apply_to_snapshot_ms": round((time.monotonic()-started)*1000, 2),
                        "marker_matches": True})
        if len(records) < len(states):
            apply_state()
        else:
            Gtk.main_quit()
    except Exception as exc:
        fail(exc)


def ready(_manager, message):
    try:
        metadata = json.loads(message.get_js_value().to_string())
        if metadata["revision"] != len(records) + 1:
            raise RuntimeError("unexpected render revision")
        view.get_snapshot(WebKit2.SnapshotRegion.VISIBLE, WebKit2.SnapshotOptions.NONE,
                          None, captured, metadata)
    except Exception as exc:
        fail(exc)


manager.connect("script-message-received::ready", ready)
view.connect("load-changed", lambda _view, event: apply_state()
             if event == WebKit2.LoadEvent.FINISHED else None)
GLib.timeout_add_seconds(30, lambda: fail("snapshot probe timed out"))
window.show_all()
view.load_uri(args.html.resolve().as_uri())
Gtk.main()
window.destroy()
report = {"backend": "WebKitGTK native get_snapshot", "webkit": ".".join(map(str, [
    WebKit2.get_major_version(), WebKit2.get_minor_version(), WebKit2.get_micro_version()])),
    "scope": "standalone visible GTK WebView; mock content; not Tauri/MCP integration",
    "captures": records, "errors": failure}
(args.output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report, indent=2))
raise SystemExit(1 if failure or len(records) != len(states) else 0)
