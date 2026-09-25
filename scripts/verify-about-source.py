"""关于面板「源码」入口的隔离实例取证（AGPL 对应源码口径的配套验收）。

用法：python3 scripts/verify-about-source.py

做法：Xvfb + 隔离 fixture 库跑**真实 WebView**，用 WebKit Inspector 的 `Runtime.evaluate`
进 DOM，断言三件事（都不是"看起来对"）：

1. 启动自检报告 `i18n selftest ok`（两份字典 key 一致，含本次新增的 3 个 key）；
2. 「关于」面板里出现源码行与按钮，zh-CN / en 两份文案都正确、按钮可见且位于该面板内；
3. 点按钮后 **Rust 侧真的用该 URL 调用了系统打开器** —— 用 PATH 前置的 `xdg-open` 桩记录参数，
   既不真的拉起浏览器，也把"点了一跳"变成可判定的证据。

证据口径：机械结论只来自 **DOM 断言 + 打开器桩参数**；面板截图仍会存一份，但它走
`import -window root`，按 `manual-verification-checklist.md` §30 **不作数**，仅供人工查看。
`window.__TAURI__.core.invoke` 实测不可写（报告里的 `invoke_writable`），所以不在 JS 层拦 invoke。

证据目录：`.chorus/specs/rss-reader/2026-09-24-license-policy/`。
"""
import asyncio
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import sqlite3
import stat
import subprocess
import sys
import tempfile

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    'native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native_probe)

SOURCE_URL = 'https://github.com/expoli/RustRss'
EVIDENCE = Path('.chorus/specs/rss-reader/2026-09-24-license-policy')
ZH = {'label': '源码', 'hint': '完整源码、问题反馈与发布说明', 'button': '打开仓库'}
EN = {'label': 'Source', 'hint': 'Full source, issue tracker and release notes', 'button': 'Open repository'}
BY_LOCALE = {'zh-CN': ZH, 'en': EN}

# 一条表达式同时读回「行文案 + 按钮文案 + 可见性 + 是否真的在该面板里」
PANEL_STATE = """JSON.stringify((()=>{const pane=document.getElementById('pane-general');
const row=pane.querySelector('[data-i18n="settings.about.source"]');
const hint=pane.querySelector('[data-i18n="settings.about.sourceHint"]');
const btn=document.getElementById('about-open-source');
const rect=btn.getBoundingClientRect();
return {label:row&&row.textContent,hint:hint&&hint.textContent,button:btn&&btn.textContent,
visible:getComputedStyle(btn).display!=='none'&&rect.width>0&&rect.height>0,
inPane:pane.contains(row)&&pane.contains(btn)};})())"""


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


async def main():
    subprocess.run(['df', '-h', '.'], check=True)
    with tempfile.TemporaryDirectory(prefix='rustrss-about-source-') as temporary:
        root = Path(temporary)
        dbpath = root / 'fixture.sqlite'
        subprocess.run(['target/debug/examples/theme_fixture', str(dbpath)], check=True)
        with sqlite3.connect(dbpath) as db:
            db.execute("UPDATE settings SET value='false' WHERE key='refresh.on_start'")
            db.execute("UPDATE settings SET value='off' WHERE key='refresh.interval_minutes'")

        # 系统打开器桩：只把参数写进文件，绝不真的拉起浏览器（也避免碰宿主桌面）
        stub = root / 'bin'
        stub.mkdir()
        calls = root / 'xdg-open.calls'
        opener = stub / 'xdg-open'
        opener.write_text('#!/bin/sh\nprintf "%s\\n" "$@" >> "$XDG_OPEN_CALLS"\n')
        opener.chmod(opener.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

        runtime = root / 'runtime'
        runtime.mkdir(mode=0o700)
        inspector_port = free_port()
        env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
                   XDG_RUNTIME_DIR=str(runtime), GDK_GL='disable', GDK_SCALE='1',
                   RUSTSS_DB=str(dbpath), RUSTSS_LOG_STDOUT='1', XDG_OPEN_CALLS=str(calls),
                   PATH=str(stub) + ':' + os.environ['PATH'],
                   WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{inspector_port}')
        for key in ('GDK_BACKEND', 'WAYLAND_DISPLAY', 'EGL_PLATFORM'):
            env.pop(key, None)

        read_fd, write_fd = os.pipe()
        xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '1400x1000x24'],
                                pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os.close(write_fd)
        app = None
        report = {'checks': [], 'expected_url': SOURCE_URL, 'opener_stub': 'xdg-open (PATH 前置)',
                  'limits': ['面板截图走 `import -window root`，按 manual-verification-checklist §30 不作数，仅人工查看',
                             '机械结论 = DOM 断言 + 打开器桩收到的参数',
                             '未验：真实桌面会话下浏览器真的打开、macOS / Windows 的打开器']}
        try:
            with os.fdopen(read_fd) as pipe:
                env['DISPLAY'] = ':' + pipe.readline().strip()
            log_path = root / 'desktop.log'
            with log_path.open('w') as log:
                app = subprocess.Popen(['target/debug/rustrss-desktop'], env=env, stdout=log, stderr=log)
            probe = native_probe.Probe({'root': str(root), 'inspector_port': inspector_port})
            for _ in range(300):
                if 'loaded feeds=' in log_path.read_text():
                    break
                assert app.poll() is None, log_path.read_text()
                await asyncio.sleep(.1)

            # 1) 启动自检：i18n key 一致性（新增 key 后仍为 0 缺 key）
            await probe.until("!!document.getElementById('btn-settings')")
            line = next((l for l in log_path.read_text().splitlines() if 'i18n selftest' in l), None)
            assert line and 'i18n selftest ok' in line, line
            report['i18n_selftest'] = line.split('] ', 1)[-1]
            report['checks'].append('启动自检报告 i18n 两份字典 key 一致')

            # 2) 打开设置 → 通用（关于面板所在页）
            await probe.js("document.getElementById('btn-settings').click();true")
            await probe.until("!!document.getElementById('tab-general')")
            await probe.js("document.getElementById('tab-general').click();true")
            await probe.until("!document.getElementById('pane-general').classList.contains('hidden')")
            # 启动 locale 由 navigator.language 解析（无头 WebKit下是 en），不写死顺序：
            # 先按当前 locale 断言，再切到另一种口径断言同一行的文案随之更新。
            startup_locale = await probe.js('window.I18N.locale()')
            assert startup_locale in BY_LOCALE, startup_locale
            other_locale = 'en' if startup_locale == 'zh-CN' else 'zh-CN'
            report['startup_locale'] = startup_locale
            first = json.loads(await probe.js(PANEL_STATE))
            expected = BY_LOCALE[startup_locale]
            assert first['label'] == expected['label'] and first['hint'] == expected['hint'], first
            assert first['button'] == expected['button'], first
            assert first['visible'] and first['inPane'], first
            report[startup_locale] = first
            report['checks'].append(
                f'关于面板出现源码行与按钮，{startup_locale} 文案与可见性符合预期')

            await probe.js("window.I18N.setLocale('%s');window.I18N.applyStaticI18n();true" % other_locale)
            second = json.loads(await probe.js(PANEL_STATE))
            other = BY_LOCALE[other_locale]
            assert second['label'] == other['label'] and second['hint'] == other['hint'], second
            assert second['button'] == other['button'], second
            report[other_locale] = second
            report['checks'].append(f'切到 {other_locale} 后同一行的文案随之更新（双语齐备）')
            await probe.js("window.I18N.setLocale('zh-CN');window.I18N.applyStaticI18n();true")

            # 3) 面板截图（无 WM + 软渲染：先制造 damage，避免陈旧帧）
            try:
                window = subprocess.check_output(['xdotool', 'search', '--onlyvisible', '--name', 'RustRss'],
                                                 env=env, text=True).split()[-1]
                subprocess.run(['xdotool', 'windowsize', window, '1239', '820'], env=env, check=True)
                subprocess.run(['xdotool', 'windowsize', window, '1240', '820'], env=env, check=True)
                await asyncio.sleep(.8)
                shot = root / 'about-source-panel.png'
                subprocess.run(['import', '-window', 'root', str(shot)], env=env, check=True)
                from PIL import Image
                image = Image.open(shot)
                image.crop((0, 0, 1240, 820)).save(shot)
                report['screenshot'] = 'about-source-panel.png'
            except Exception as error:  # 截图失败不影响 DOM/调用断言，但要如实记录
                report['screenshot_error'] = repr(error)

            # 4) 点击按钮：以 OS 层证据为准（PATH 前置的 xdg-open 桩），并做点击前后的因果对照。
            # 不在 JS 层拦 invoke：window.__TAURI__.core.invoke 可能不可写，patch 会被静默忽略。
            assert not (calls.exists() and calls.read_text().strip()), '点击前打开器不应被调用'
            report['handler_type'] = await probe.js(
                "typeof document.getElementById('about-open-source').onclick")
            assert report['handler_type'] == 'function', report['handler_type']
            report['invoke_writable'] = await probe.js(
                "(()=>{try{const d=Object.getOwnPropertyDescriptor(window.__TAURI__.core,'invoke');"
                "return !!d&&!!d.writable}catch(error){return false}})()")
            await probe.js("document.getElementById('about-open-source').click();true")
            for _ in range(60):
                if calls.exists() and calls.read_text().strip():
                    break
                await asyncio.sleep(.1)
            opened = calls.read_text().split() if calls.exists() else []
            assert opened and opened[-1] == SOURCE_URL, opened
            report['opener_arguments'] = opened
            report['checks'].append('点击按钮：打开器被以该 URL 调用（点击前无调用，因果明确）')

            report['passed'] = True
            EVIDENCE.mkdir(parents=True, exist_ok=True)
            (EVIDENCE / 'about-source-results.json').write_text(
                json.dumps(report, ensure_ascii=False, indent=2) + '\n')
            if 'screenshot' in report:
                shutil.copyfile(root / report['screenshot'], EVIDENCE / report['screenshot'])
            print(json.dumps({'checks': report['checks'], 'passed': True,
                              'evidence': str(EVIDENCE)}, ensure_ascii=False), flush=True)
        finally:
            if app and app.poll() is None:
                app.terminate()
                app.wait(timeout=10)
            xvfb.terminate()
            xvfb.wait(timeout=10)


asyncio.run(main())
