"""离线场景检查（T2 AC3）：隔离网络命名空间（仅 lo）下带缩略图的列表可读可滚、图片失败不影响正文。

用法（**必须**在独立网络命名空间里跑，否则测的不是离线）：

    unshare -rn bash -c 'cd <repo 根目录> && python3 scripts/verify-thumbnail-offline.py <证据目录>'

两个坑已在脚本里自理：
- 新 netns 里 `lo` 默认 **DOWN**（不拉起会 Network is unreachable）——脚本先 `bring_lo_up()`；
- 应用的 WebKit inspector 绑在环回上，命名空间外连不到，所以整个探针都跑在命名空间里。
`offline_guard()` 随后断言：只有 lo **且 lo 已 UP**，否则拒绍出结论。

断言（离线＝只有 lo、且缩略图指向公网主机 → 必然失败）：
1. 列表渲染出条目（离线可读）；
2. 缩略图节点存在但 `naturalWidth == 0`（失败静默，不是空白占位）；
3. 无阻塞式 dialog；
4. 正文可读：点开第一条后正文面板有文本；
5. 列表可滚动；
6. 全过程行 id 不重建。
"""
import asyncio
import importlib.util
import json
import os
from pathlib import Path
import socket
import sqlite3
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    'native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native_probe)

# 公网主机：离线命名空间里必然连不上（用于制造「图片失败」）
UNREACHABLE_THUMBNAIL = 'https://raw.githubusercontent.com/github/explore/main/topics/rust/rust.png'
OUT_DIR = Path(sys.argv[1]) if len(sys.argv) > 1 else None


def write_evidence(name, payload):
    if OUT_DIR is None:
        return None
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    path = OUT_DIR / name
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2) + '\n')
    return str(path)


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def bring_lo_up():
    """新 netns 里 `lo` 默认是 **DOWN** 的：不先拉起，连本地 inspector/DNS 都会报 Network is unreachable
    （实测：只跑 `unshare -rn python3 …` 直接崩）。脚本自己拉起，这样文档里那一行命令就是完整可复现的。"""
    subprocess.run(['ip', 'link', 'set', 'lo', 'up'], check=True, timeout=10)


def offline_guard():
    """确认真的在「只有 lo 且 lo 已 UP」的网络命名空间里。

    只看接口名不够：fresh netns 里 lo 也存在但 DOWN（此时根本没网，测出来的「离线」不是我们要的场景）。
    有非 lo 接口、或 lo 未 UP，都直接失败。
    """
    out = subprocess.run(['ip', '-o', 'link', 'show'], capture_output=True, text=True, timeout=10).stdout
    ifaces = [line.split(':')[1].strip() for line in out.splitlines() if ':' in line]
    others = [i for i in ifaces if i != 'lo']
    assert not others, f'命名空间里存在非 lo 接口 {others}：这不是离线环境，拒绝出结论'
    lo_line = next((line for line in out.splitlines() if ': lo:' in line), '')
    assert 'UP' in lo_line, f'lo 未 UP（新 netns 默认 DOWN），先 bring_lo_up: {lo_line!r}'


async def main():
    bring_lo_up()
    offline_guard()
    report = {'unreachable_thumbnail': UNREACHABLE_THUMBNAIL,
              'checked_at': datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
              'network': 'private netns, lo only', 'checks': []}
    with tempfile.TemporaryDirectory(prefix='rustrss-thumbnail-offline-') as temporary:
        root = Path(temporary)
        dbpath = root / 'fixture.sqlite'
        subprocess.run(['target/debug/examples/theme_fixture', str(dbpath)], check=True)
        with sqlite3.connect(dbpath) as db:
            ids = [row[0] for row in db.execute(
                'SELECT id FROM entries ORDER BY COALESCE(published_at, fetched_at) DESC, id DESC LIMIT 3')]
            db.execute('UPDATE entries SET thumbnail_url=?', (UNREACHABLE_THUMBNAIL,))
            db.execute("UPDATE settings SET value='false' WHERE key='refresh.on_start'")
            db.execute("UPDATE settings SET value='off' WHERE key='refresh.interval_minutes'")
        report['entry_ids'] = ids

        runtime = root / 'runtime'
        runtime.mkdir(mode=0o700)
        inspector_port = free_port()
        env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
                   XDG_RUNTIME_DIR=str(runtime), GDK_GL='disable', GDK_SCALE='1', RUSTSS_DB=str(dbpath),
                   WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{inspector_port}')
        for key in ('GDK_BACKEND', 'WAYLAND_DISPLAY', 'EGL_PLATFORM'):
            env.pop(key, None)
        read_fd, write_fd = os.pipe()
        xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '1400x1000x24'],
                                pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os.close(write_fd)
        app = None
        try:
            with os.fdopen(read_fd) as pipe:
                env['DISPLAY'] = ':' + pipe.readline().strip()
            log_path = root / 'desktop.log'
            with log_path.open('w') as log:
                app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log,
                                       start_new_session=True)
            probe = native_probe.Probe({'root': str(root), 'inspector_port': inspector_port})
            for _ in range(200):
                if 'loaded feeds=' in log_path.read_text():
                    break
                assert app.poll() is None, log_path.read_text()[-1500:]
                await asyncio.sleep(.1)
            report['app_log_tail'] = log_path.read_text()[-300:]

            # 1) 离线仍渲染出列表
            await probe.until("!!document.querySelector('#entries li[data-id]')")
            await probe.until('document.querySelectorAll(\'#entries li[data-id]\').length>0')
            await probe.js("window.__thumbEnable={done:false};window.__TAURI__.core.invoke('get_theme_update',{knownRevision:null}).then(snapshot=>window.__TAURI__.core.invoke('update_ui_theme',{expectedRevision:snapshot.config.revision,patch:{overrides:{list:{thumbnail:true}}}})).then(()=>window.__thumbEnable.done=true,error=>window.__thumbEnable.error=String(error));true")
            await probe.until('window.__thumbEnable.done===true')
            await probe.until("document.documentElement.dataset.thumbnails==='true'")
            report['checks'].append('list renders offline (entries visible)')

            # 2) 缩略图失败静默（节点在、naturalWidth==0、无 dialog）
            await asyncio.sleep(12)
            state = await probe.js(
                "(()=>{const n=document.querySelector('#entries .entry-thumbnail');"
                "return {present:!!n,width:n?n.naturalWidth:null,complete:n?n.complete:null,"
                "dialog:!!document.querySelector('dialog[open]')}})()")
            report['thumbnail_state'] = state
            assert state['present'], '缩略图节点应存在（配置开着）'
            assert state['width'] == 0, f"离线时缩略图应加载失败: {state}"
            assert not state['dialog'], '离线时不应出现阻塞式对话框'
            report['checks'].append('offline thumbnails fail silently without dialogs')

            # 3) 正文可读：打开第一条
            await probe.js("document.querySelector('#entries li[data-id]').click();true")
            await probe.until("!!document.querySelector('#article, #content, article')")
            await asyncio.sleep(1)
            body = await probe.js(
                "(()=>{const el=document.querySelector('#article, #content, article');"
                "return el?el.innerText.trim().length:0})()")
            report['article_text_length'] = body
            assert body and body > 0, '离线时应能读到已缓存正文'
            report['checks'].append('cached article body is readable offline')

            # 4) 可滚动 + 行不重建
            before = await probe.js("Array.from(document.querySelectorAll('#entries li[data-id]')).map(li=>li.dataset.id).join(',')")
            await probe.js("(()=>{const l=document.getElementById('entries');l.scrollTop=l.scrollHeight;return true})()")
            await asyncio.sleep(1)
            after = await probe.js("Array.from(document.querySelectorAll('#entries li[data-id]')).map(li=>li.dataset.id).join(',')")
            scrolled = await probe.js("document.getElementById('entries').scrollTop>0")
            report['row_ids_unchanged'] = before == after
            report['scrolled'] = scrolled
            assert before == after, '离线时不应重建列表行'
            assert scrolled, '离线时应仍可滚动'
            report['checks'].append('offline list scrolls and does not rebuild rows')
            report['evidence_file'] = write_evidence('thumbnail-offline-results.json', report)
            print(json.dumps(report, ensure_ascii=False))
        finally:
            if app and app.poll() is None:
                app.terminate()
                try:
                    app.wait(timeout=8)
                except subprocess.TimeoutExpired:
                    app.kill()
                    app.wait(timeout=5)
            xvfb.terminate()
            xvfb.wait(timeout=10)


asyncio.run(main())
