import pathlib, subprocess, time, xml.etree.ElementTree as ET, re, json, hashlib

adb = '/usr/lib/android-sdk/platform-tools/adb'
serial = 'emulator-5580'
pkg = 'tech.expoli.rustrss'
old = '/tmp/rust/upgrade-apks/RustRss_0.2.1_android-universal.apk'
new = '/tmp/rust/upgrade-apks/RustRss_0.3.1_android-universal.apk'
OLD_PUBLISHED_SHA = 'b0e9474f7364b3713195e080ed87b99fb0cf29e6e09345e3aef6394076a8b6c1'
NEW_PUBLISHED_SHA = '99e666885ea495d94379028d359d5bf91147e6cb798f849d449586213f638bb2'
out = pathlib.Path('/home/tcy/Github/RustRss/docs/releases/v0.3.1-android')
out.mkdir(parents=True, exist_ok=True)
FEED_A, FEED_B = 'Upgrade Fixture A', 'Upgrade Fixture B'

def run(*a, timeout=60):
    return subprocess.check_output([adb, '-s', serial, *a], timeout=timeout)

def install(apk):
    return run('install', '-r', apk).decode().strip()

def tree():
    for _ in range(5):
        dumped = run('shell', 'uiautomator', 'dump', '/data/local/tmp/upg.xml').decode()
        if 'dumped to:' in dumped:
            return ET.fromstring(run('shell', 'cat', '/data/local/tmp/upg.xml'))
        time.sleep(0.3)
    raise AssertionError('No native UI root')

def touch(n):
    x1, y1, x2, y2 = map(int, re.findall(r'\d+', n.get('bounds')))
    assert x2 > x1 and y2 > y1
    run('shell', 'input', 'tap', str((x1 + x2) // 2), str((y1 + y2) // 2))
    time.sleep(0.6)

def tap(rid=None, text=None, rounds=6):
    for _ in range(rounds):
        nodes = list(tree().iter('node'))
        n = next((n for n in nodes if (n.get('resource-id') == rid if rid else n.get('text') == text)), None)
        if n is not None:
            touch(n)
            return n
        time.sleep(0.4)
    raise AssertionError(f'not found: rid={rid} text={text}')

def wait_pkg_top():
    for _ in range(20):
        current = run('shell', 'dumpsys', 'activity', 'activities').decode()
        top = next((l for l in current.splitlines() if 'topResumedActivity' in l), '')
        if pkg in top:
            return
        time.sleep(0.3)
    raise AssertionError('RustRss not top activity')

def screenshot(name):
    (out / (name + '.png')).write_bytes(run('exec-out', 'screencap', '-p'))

def uid():
    return re.search(r'uid:(\d+)', run('shell', 'pm', 'list', 'packages', '-U', pkg).decode()).group(1)

def launch():
    run('shell', 'am', 'start', '-n', pkg + '/.MainActivity')
    time.sleep(2.5)
    wait_pkg_top()

def has_feed(title):
    for _ in range(10):
        if any(n.get('text') == title or n.get('text', '').startswith(title + ' ')
               for n in tree().iter('node')):
            return True
        time.sleep(0.4)
    return False

def add_feed_by_url(url):
    tap(text='Subscriptions')
    tap(rid='add-url')
    run('shell', 'input', 'text', url)
    time.sleep(0.6)
    tap(rid='add-ok')
    time.sleep(2)
    tap(text='Subscriptions')
    time.sleep(0.5)

def launcher_launch(name):
    run('shell', 'input', 'keyevent', 'KEYCODE_HOME')
    time.sleep(0.6)
    run('shell', 'input', 'swipe', '540', '2200', '540', '600', '400')
    time.sleep(0.8)
    for _ in range(10):
        nodes = list(tree().iter('node'))
        n = next((n for n in nodes if n.get('text') == 'RustRss'), None)
        if n is not None:
            screenshot(name)
            touch(n)
            time.sleep(1.5)
            wait_pkg_top()
            return True
        run('shell', 'input', 'swipe', '540', '1800', '540', '700', '350')
        time.sleep(0.6)
    raise AssertionError('RustRss launcher entry unavailable')

def installed_version_and_hash():
    info = run('shell', 'dumpsys', 'package', pkg).decode()
    vn = re.search(r'versionName=(\S+)', info).group(1)
    vc = int(re.search(r'versionCode=(\d+)', info).group(1))
    path = run('shell', 'pm', 'path', pkg).decode().strip().removeprefix('package:')
    digest = hashlib.sha256(run('exec-out', 'cat', path)).hexdigest()
    return vn, vc, digest

result = {}

# --- 0. clean slate: remove whatever build is on the AVD (dev-signed builds block install)
pre = run('shell', 'dumpsys', 'package', pkg).decode()
pre_m = re.search(r'versionName=(\S+)', pre)
if pre_m:
    result['pre_existing_removed'] = {'versionName': pre_m.group(1),
                                      'versionCode': int(re.search(r'versionCode=(\d+)', pre).group(1))}
    print('removing pre-existing build', result['pre_existing_removed'], flush=True)
    run('uninstall', pkg)

# --- 1. install published 0.2.1, verify installed bytes == published asset
print(install(old), flush=True)
vn, vc, digest = installed_version_and_hash()
assert (vn, vc) == ('0.2.1', 2001), (vn, vc)
assert digest == OLD_PUBLISHED_SHA, digest
result['published_021_hash_verified'] = True
print('v0.2.1 installed, hash matches published asset', flush=True)

# --- 2. seed two subscriptions via URL add (host HTTP server via 10.0.2.2, real fetch+parse)
launch()
add_feed_by_url('http://10.0.2.2:18080/a.xml')
add_feed_by_url('http://10.0.2.2:18080/b.xml')
assert has_feed(FEED_A) and has_feed(FEED_B), 'fixtures missing after add'
uid_old = uid()
screenshot('v031-seeded-021-data')
result.update(uid_021=uid_old, two_fixtures_021=True)
print('seeded two fixture feeds on 0.2.1, uid', uid_old, flush=True)

# --- 3. upgrade in place: install -r 0.3.1
run('shell', 'am', 'force-stop', pkg)
print(install(new), flush=True)
vn, vc, digest = installed_version_and_hash()
assert (vn, vc) == ('0.3.1', 3001), (vn, vc)
assert digest == NEW_PUBLISHED_SHA, digest
uid_new = uid()
result.update(uid_031=uid_new, same_uid=uid_old == uid_new,
              version_name=vn, version_code=vc, higher_version_code=vc > 2001)
assert uid_old == uid_new, 'UID changed across upgrade'
print('upgraded to 0.3.1/3001, same uid', uid_new, flush=True)

# --- 4. launcher icon launch
assert launcher_launch('v031-launcher')
result['new_launcher_launch'] = True

# --- 5. retained data on Subscriptions tab
tap(text='Subscriptions')
assert has_feed(FEED_A) and has_feed(FEED_B), 'feeds lost across upgrade'
screenshot('v031-retained-data')
result['two_fixtures_retained_031'] = True
print('both fixture feeds retained on 0.3.1', flush=True)

# --- 6. warm IME: URL entry stays above keyboard
tap(rid='add-url')
tap(rid='add-url')
run('shell', 'input', 'text', 'https://upgrade.invalid/rss')
time.sleep(0.8)
ime = run('shell', 'dumpsys', 'input_method').decode()
assert 'mInputShown=true' in ime or 'mIsInputViewShown=true' in ime, 'IME not shown'
n = next(n for n in tree().iter('node') if n.get('resource-id') == 'add-url')
y1, y2 = map(int, re.findall(r'\d+', n.get('bounds'))[1::2])
assert y2 < 900, (y1, y2)
screenshot('v031-ime-visible-entry')
run('shell', 'input', 'keyevent', 'KEYCODE_BACK')
time.sleep(0.4)
result['warm_ime_input_visible'] = True
print('warm IME check passed, entry y2 =', y2, flush=True)

# --- 7. settings home renders category list
tap(text='Settings')
assert any(n.get('resource-id') == 'tab-reading' for n in tree().iter('node'))
screenshot('v031-settings-home')
result['phone_settings_home'] = True

(out / 'results.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, indent=2), flush=True)
print('ALL CHECKS PASSED', flush=True)
