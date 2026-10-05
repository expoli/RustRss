"""Rebuilt Linux WebKitGTK app, isolated fixture and Xvfb; inspector DOM actions + real IPC.
No claim of native pointer, Android/physical-device or Windows/macOS verification.
"""
import asyncio
import importlib.util
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import tempfile

spec = importlib.util.spec_from_file_location('probe', 'scripts/verify-theme-native-settings.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
output = Path(__file__).resolve().parent

async def main():
    root = Path(tempfile.mkdtemp(prefix='rustrss-tags-b-prime-'))
    for folder in ['home', 'data', 'config', 'cache', 'runtime']:
        (root / folder).mkdir(mode=0o700)
    # Snapshot, never attach to the user's database or mutate the benchmark input.
    src = sqlite3.connect('target/verification-followups/effective.sqlite')
    dst = sqlite3.connect(root / 'fixture.sqlite')
    src.backup(dst)
    dst.close()
    src.close()
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        port = s.getsockname()[1]
    read, write = os.pipe()
    xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write), '-screen', '0', '1240x900x24'], pass_fds=(write,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    os.close(write)
    with os.fdopen(read) as p:
        display = ':' + p.readline().strip()
    env = dict(os.environ, DISPLAY=display, GDK_GL='disable', HOME=str(root/'home'),
        XDG_DATA_HOME=str(root/'data'), XDG_CONFIG_HOME=str(root/'config'), XDG_CACHE_HOME=str(root/'cache'),
        XDG_RUNTIME_DIR=str(root/'runtime'), RUSTSS_DB=str(root/'fixture.sqlite'), RUSTSS_LOG_STDOUT='1',
        RUSTSS_AI_KEY='isolated-fixture-placeholder', WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{port}')
    for key in ['WAYLAND_DISPLAY', 'GDK_BACKEND', 'QT_QPA_PLATFORM']:
        env.pop(key, None)
    log = (root/'desktop.log').open('w')
    app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log)
    probe = module.Probe({'root': str(root), 'inspector_port': port})
    try:
        for _ in range(200):
            if 'loaded feeds=' in (root/'desktop.log').read_text():
                break
            assert app.poll() is None
            await asyncio.sleep(.1)
        await probe.until("document.querySelectorAll('#entries li[data-id]').length > 0")
        async def click(selector):
            await probe.js("document.querySelector("+json.dumps(selector)+").click(); true")
        async def picker_for_article(entry):
            await click(f'#entries li[data-id="{entry}"]')
            await probe.until("!!document.querySelector('#reader-tags .tag-add')")
            await click('#reader-tags .tag-add')
            await probe.until("!document.getElementById('tag-picker-overlay').classList.contains('hidden')")
        async def feed_picker(feed):
            await click(f'#feeds li[data-feed-id="{feed}"] .row-more')
            await probe.until("!!document.querySelector('#ctx-menu')")
            await probe.js("[...document.querySelectorAll('#ctx-menu > button')].find(b=>b.textContent.includes(I18N.t('menu.feedTags'))).click(); true")
            await probe.until("!document.getElementById('tag-picker-overlay').classList.contains('hidden')")
        await click('#feeds li[data-feed-id="1"] .name')
        await probe.until("!!document.querySelector('#entries li[data-id=\"100\"]')")
        await picker_for_article(100)
        both = await probe.js("({disabled:document.querySelector('#tag-picker-list input').disabled,title:document.querySelector('#tag-picker-list li').title,text:document.querySelector('#tag-picker-list li').textContent})")
        assert both['disabled'] and '手动＋源继承' in both['text'] and both['title'] == '请在订阅源中修改继承标签', both
        await click('#tag-picker-list li') # readonly item: same delegated path as mouse/Enter
        await click('#tag-picker-close')
        await click('#tags li[data-tag-id="1"] .name')
        await probe.until("!!document.querySelector('#entries li[data-id=\"6000\"]')")
        await picker_for_article(6000)
        inherited = await probe.js("({disabled:document.querySelector('#tag-picker-list input').disabled,title:document.querySelector('#tag-picker-list li').title,text:document.querySelector('#tag-picker-list li').textContent})")
        assert inherited['disabled'] and '源继承' in inherited['text'], inherited
        await probe.js("I18N.setLocale('en'); document.getElementById('tag-picker-input').dispatchEvent(new Event('input',{bubbles:true})); true")
        english = await probe.js("({title:document.querySelector('#tag-picker-list li').title,text:document.querySelector('#tag-picker-list li').textContent})")
        assert english['title'] == 'Edit inherited tags on the feed' and 'From feed' in english['text'], english
        assert (await probe.js('I18N.selfTest()'))['ok']
        await probe.js("window.__kept=document.querySelector('#entries li[data-id=\"5900\"]'); window.__reader=document.getElementById('reader').firstElementChild; true")
        assert await probe.js('!!window.__reader && !!window.__kept')
        await click('#tag-picker-close')
        await feed_picker(60)
        await click('#tag-picker-list li')
        await probe.until("document.querySelector('#tags li[data-tag-id=\"1\"] .count').textContent === '5899'")
        await probe.until("document.querySelector('#entries li[data-id]')?.dataset.id === '5900'")
        await probe.until("document.getElementById('list-count').textContent.includes('5900')")
        after = await probe.js("({countText:document.getElementById('list-count').textContent,unread:document.querySelector('#tags li[data-tag-id=\"1\"] .count').textContent,rows:document.querySelectorAll('#entries li[data-id]').length,head:Number(document.querySelector('#entries li[data-id]').dataset.id),kept:__kept===document.querySelector('#entries li[data-id=\"5900\"]'),reader:__reader===document.getElementById('reader').firstElementChild,chips:document.querySelectorAll('#reader-tags .tag-chip').length})")
        assert after['unread']=='5899' and after['rows']==200 and after['head']==5900 and after['kept'] and after['reader'] and after['chips']==0, after
        with sqlite3.connect(root/'fixture.sqlite') as c:
            assert c.execute('SELECT count(*) FROM feed_tags WHERE feed_id=60').fetchone()[0] == 0
            assert c.execute('SELECT count(*) FROM entry_tags WHERE entry_id=100 AND tag_id=1').fetchone()[0] == 1
        await click('#tag-picker-close')
        await feed_picker(1)
        await click('#tag-picker-list li')
        await probe.until("!document.querySelector('#tag-picker-list input').checked")
        await click('#tag-picker-close')
        await click('#feeds li[data-feed-id="1"] .name')
        await probe.until("!!document.querySelector('#entries li[data-id=\"100\"]')")
        await picker_for_article(100)
        manual = await probe.js("({disabled:document.querySelector('#tag-picker-list input').disabled,text:document.querySelector('#tag-picker-list li').textContent})")
        assert not manual['disabled'] and 'From feed' not in manual['text'], manual
        result = {'passed':True,'root':str(root),'binary':'target/debug/rustrss-desktop','both_zh':both,'feed_zh':inherited,'feed_en':english,'after_feed_removal':after,'manual_after_feed_removal':manual,
            'limits':['Inspector DOM actions, not native pointer input', 'Linux Xvfb/WebKitGTK only; mobile/narrow-screen and other platforms unverified']}
        (output/'ui.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
        (output/'ui.log').write_text((root/'desktop.log').read_text())
        print(json.dumps(result,ensure_ascii=False))
    finally:
        for p in [app,xvfb]:
            if p.poll() is None:
                p.terminate()
                p.wait(timeout=10)
        log.close()

asyncio.run(main())
