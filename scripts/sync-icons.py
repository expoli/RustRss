#!/usr/bin/env python3
"""Regenerate packaged icons from the shared SVG, or check committed provenance."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / 'src-tauri/icons'
SOURCE = ICONS / 'src/rustrss-icon.svg'
NATIVE = ROOT / 'src-tauri/gen/android/app/src/main/res'
MANIFEST = ROOT / 'src-tauri/gen/android/app/src/main/AndroidManifest.xml'
RECORD = ICONS / 'provenance.json'
NS = '{http://schemas.android.com/apk/res/android}'


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check():
    errors = []
    for path in (ICONS / 'android').rglob('*'):
        if path.is_file():
            native = NATIVE / path.relative_to(ICONS / 'android')
            if not native.exists() or native.read_bytes() != path.read_bytes():
                errors.append(f'Android resource differs: {native.relative_to(ROOT)}')
    app = ET.parse(MANIFEST).getroot().find('application')
    if app.get(NS + 'icon') != '@mipmap/ic_launcher' or app.get(NS + 'roundIcon') != '@mipmap/ic_launcher_round':
        errors.append('Android manifest must reference regular and round launcher resources')
    if not (NATIVE / 'mipmap-anydpi-v26/ic_launcher.xml').exists():
        errors.append('Android adaptive launcher resource is missing')
    for old in ['drawable-v24/ic_launcher_foreground.xml', 'drawable/ic_launcher_background.xml']:
        if (NATIVE / old).exists():
            errors.append(f'Stale template resource: {old}')
    if not RECORD.exists():
        errors.append('Icon provenance is missing; regenerate from the canonical SVG')
    else:
        record = json.loads(RECORD.read_text())
        current = {str(p.relative_to(ROOT)) for p in ICONS.rglob('*')
                   if p.is_file() and p.suffix in ('.png', '.ico', '.icns')}
        expected = {p for p in record['sha256'] if p.endswith(('.png', '.ico', '.icns'))}
        if current != expected:
            errors.append('Generated icon inventory differs from provenance')
        for name, expected_hash in record['sha256'].items():
            path = ROOT / name
            if not path.exists() or digest(path) != expected_hash:
                errors.append(f'Icon/source changed without regeneration: {name}')
    if errors:
        raise SystemExit('\n'.join(errors))
    print('All platform icon hashes, Android copies and launcher references match')


def regenerate():
    with tempfile.TemporaryDirectory(prefix='rustrss-icons-') as tmp:
        work = Path(tmp)
        shutil.copy2(SOURCE, work / 'icon.svg')
        # Native vector geometry: keep the shared motif within the 66dp circle
        # on Android's 108dp adaptive canvas. The launcher masks the background.
        svg = ET.parse(SOURCE).getroot()
        background = svg.find('{http://www.w3.org/2000/svg}rect')
        assert background is not None
        svg.remove(background)
        group = ET.Element('{http://www.w3.org/2000/svg}g', {
            'transform': 'translate(256 256) scale(0.60) translate(-256 -256)'})
        for element in list(svg):
            svg.remove(element)
            group.append(element)
        svg.append(group)
        ET.ElementTree(svg).write(work / 'foreground.svg', encoding='unicode')
        (work / 'manifest.json').write_text(json.dumps({
            'default': 'icon.svg', 'android_fg': 'foreground.svg',
            'bg_color': '#1c1f26'}))
        out = work / 'icons'
        subprocess.run(['cargo', 'tauri', 'icon', str(work / 'manifest.json'),
                        '--output', str(out)], cwd=ROOT, check=True)
        subprocess.run(['cargo', 'tauri', 'icon', str(SOURCE), '--png', '64',
                        '--output', str(work / 'extra')], cwd=ROOT, check=True)
        shutil.copy2(work / 'extra/64x64.png', out / '64x64.png')
        shutil.copy2(out / '128x128@2x.png', out / 'icon-256.png')
        adaptive = out / 'android/mipmap-anydpi-v26/ic_launcher.xml'
        shutil.copy2(adaptive, adaptive.with_name('ic_launcher_round.xml'))
        # Copy only generated files: preserve the canonical SVG and design notes.
        for path in out.rglob('*'):
            if path.is_file():
                target = ICONS / path.relative_to(out)
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(path, target)
        for path in (out / 'android').rglob('*'):
            if path.is_file():
                target = NATIVE / path.relative_to(out / 'android')
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(path, target)
        # These unused stock Tauri vectors must not survive future references.
        for old in ['drawable-v24/ic_launcher_foreground.xml', 'drawable/ic_launcher_background.xml']:
            (NATIVE / old).unlink(missing_ok=True)
        files = [SOURCE, Path(__file__).resolve()]
        files += [p for p in ICONS.rglob('*') if p.is_file()
                  and p.suffix in ('.png', '.ico', '.icns')]
        files += [p for p in (ICONS / 'android').rglob('*.xml')]
        RECORD.write_text(json.dumps({
            'generator': subprocess.check_output(['cargo', 'tauri', '--version'], text=True).strip(),
            'source': str(SOURCE.relative_to(ROOT)), 'adaptive_scale': 0.60,
            'sha256': {str(p.relative_to(ROOT)): digest(p) for p in sorted(files)}
        }, indent=2) + '\n')
    check()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    check() if args.check else regenerate()
