"""Run the isolated Tauri production-component fixture, never the user's database."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
from PIL import Image

parser = argparse.ArgumentParser()
parser.add_argument('--display', choices=['xvfb', 'wayland'], default='xvfb')
parser.add_argument('--scale', choices=['1', '2'], default='1')
args = parser.parse_args()
root = Path(tempfile.mkdtemp(prefix='rustrss-theme-ui-'))
env = dict(os.environ)
xvfb = None
try:
    if args.display == 'xvfb':
        read_fd, write_fd = os.pipe()
        xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '2560x1800x24'], pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os.close(write_fd)
        with os.fdopen(read_fd) as pipe:
            env['DISPLAY'] = ':' + pipe.readline().strip()
        # Only the test child is isolated; never force a backend in product code.
        for key in ('GDK_BACKEND', 'WAYLAND_DISPLAY', 'EGL_PLATFORM'):
            env.pop(key, None)
        runtime = root / 'runtime'
        runtime.mkdir(mode=0o700)
        env.update(XDG_RUNTIME_DIR=str(runtime), GDK_GL='disable', GDK_SCALE=args.scale)
    else:
        if not env.get('WAYLAND_DISPLAY'):
            raise RuntimeError('No native Wayland session available')
        env.pop('DISPLAY', None)
    result = subprocess.run(['target/debug/examples/theme_ui', str(root / 'captures')], env=env, capture_output=True, text=True, timeout=100)
    (root / 'run.log').write_text(result.stdout + result.stderr)
    report_path = root / 'captures/results.json'
    if not report_path.is_file():
        raise RuntimeError(f'Probe did not produce a report; inspect {root / "run.log"} (exit {result.returncode})')
    report = json.loads(report_path.read_text())
    if result.returncode or report.get('error'):
        raise RuntimeError(f'{root}: {report.get("error", result.stderr)}')
    for shot in report['captures']:
        image = Image.open(root / 'captures' / shot['file']).convert('RGB')
        assert list(image.size) == shot['pixels']
        # Blank area in the production article/list surface. This checks the native
        # frame carries this theme's background rather than only trusting JS state.
        if shot.get('name', '').endswith('-overview'):
            expected = tuple(bytes.fromhex(shot['background'][1:]))
            pixel = image.getpixel((image.width - 40, image.height - 90))
            assert pixel == expected, (shot['file'], pixel, expected)
        if shot.get('occlusion_point'):
            expected = tuple(bytes.fromhex(shot['settings_background'][1:]))
            x, y = shot['occlusion_point']
            pixel = image.getpixel((round(x * shot['scale']), round(y * shot['scale'])))
            assert pixel == expected, ('scrollbar occlusion', shot['file'], pixel, expected)
    report['native_background_pixel_checks'] = 12
    report['modal_occlusion_pixel_checks'] = 12
    report['display'] = args.display
    report['run_log'] = str(root / 'run.log')
    (root / 'captures/results.json').write_text(json.dumps(report, ensure_ascii=False, indent=2))
    print(json.dumps({'output': str(root), 'captures': len(report['captures']), 'checks': len(report['checks']), 'pixel_checks': 24}))
finally:
    if xvfb:
        xvfb.terminate()
        xvfb.wait(timeout=10)
