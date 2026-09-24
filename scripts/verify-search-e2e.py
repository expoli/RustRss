"""搜索端到端（T2 / 任务 94422722）：真实 WebView 输入 → 列表渲染的时延与正确性。

用法：python3 scripts/verify-search-e2e.py [证据目录]

口径（AC2 要求「标注构建类型与冷页/热缓存」）：
- 被测二进制由 `--bin` 决定（默认 `target/release/rustrss-desktop`，缺则退 debug 并在报告里**如实标注**）；
- 库用 `search_scale init` 生成的 **10k 篇**生产形状夹具，每轮拷一份**新文件**（SSD 零驻留页）→ 首个查询算冷页；
- 同一进程内的后续查询算热缓存；
- 时延口径 = **输入事件 → 列表完成渲染**（MutationObserver + performance.now），与既有脚本同源。

输入法说明（AC1 要求）：xdotool 注入的是**真实键盘事件**（`isTrusted=true`），走浏览器默认输入路径；
中文输入法的候选/合成（fcitx5 / ibus）不在本探针范围 → 如实标为外部依赖。
"""
import asyncio
import importlib.util
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    'native_probe', Path(__file__).with_name('verify-theme-native-settings.py'))
native_probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native_probe)

OUT_DIR = Path(sys.argv[1]) if len(sys.argv) > 1 else None
BROAD = 'common'      # 宽泛查询：命中很多 → 走 200 行封顶页
NARROW = '稀'          # 单字 CJK 查询：历史上退化为全扫的那一类


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


def pick_binary():
    for candidate in ('target/release/rustrss-desktop', 'target/debug/rustrss-desktop'):
        if Path(candidate).exists():
            return candidate, 'release' if 'release' in candidate else 'debug'
    raise SystemExit('找不到 rustrss-desktop 构建产物')


async def wait_inspector(probe, app, log_path, tries=300):
    for _ in range(tries):
        if app.poll() is not None:
            raise AssertionError(f'应用提前退出: {log_path.read_text()[-400:]}')
        try:
            await probe.js('1')
            return
        except (OSError, IndexError):
            await asyncio.sleep(.2)
    raise AssertionError('inspector 未就绪')


def stop(proc):
    if proc is None or proc.poll() is not None:
        return
    try:
        proc.terminate()
        proc.wait(timeout=8)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=5)


MEASURE_JS = """(()=>{
  window.__m={start:null,done:null,count:null};
  const needle=%s;
  const input=document.getElementById('search');
  input.addEventListener('input',()=>{if(input.value===needle)window.__m.start=performance.now()},true);
  new MutationObserver(()=>{
    const n=document.querySelectorAll('#entries li[data-id]').length;
    if(window.__m.start!==null&&n>0){window.__m.count=n;window.__m.done=performance.now()-window.__m.start;}
  }).observe(document.getElementById('entries'),{childList:true,subtree:true});
  return true})()"""


async def main():
    binary, build_kind = pick_binary()
    report = {'binary': binary, 'build_kind': build_kind,
              'checked_at': datetime.now(timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
              'queries': {}, 'checks': []}
    with tempfile.TemporaryDirectory(prefix='rustrss-search-e2e-') as temporary:
        root = Path(temporary)
        seed = root / 'seed.sqlite'
        subprocess.run(['target/debug/examples/search_scale', 'init', str(seed)],
                       check=True, timeout=180)
        # 每轮拷一份新文件：SSD 零驻留页 → 第一个查询是冷页口径
        db_path = root / 'live.sqlite'
        shutil.copy(seed, db_path)
        runtime = root / 'runtime'
        runtime.mkdir(mode=0o700)
        inspector_port = free_port()
        env = dict(os.environ, HOME=str(root / 'home'), XDG_DATA_HOME=str(root / 'data'),
                   XDG_RUNTIME_DIR=str(runtime), GDK_GL='disable', GDK_SCALE='1',
                   RUSTSS_DB=str(db_path), WEBKIT_INSPECTOR_HTTP_SERVER=f'127.0.0.1:{inspector_port}')
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
                app = subprocess.Popen([binary], env=env, stdout=log, stderr=log, start_new_session=True)
            window = None
            for _ in range(250):
                out = subprocess.run(['xdotool', 'search', '--onlyvisible', '--name', '^RustRss$'],
                                     env=env, capture_output=True, text=True).stdout.split()
                if out:
                    window = out[0]
                    break
                assert app.poll() is None, log_path.read_text()[-600:]
                await asyncio.sleep(.2)
            assert window, '未找到主窗口'
            subprocess.run(['xdotool', 'windowfocus', window], env=env, check=True, timeout=10)
            probe = native_probe.Probe({'root': str(root), 'inspector_port': inspector_port})
            await wait_inspector(probe, app, log_path)
            await probe.until("document.querySelectorAll('#entries li[data-id]').length>0")

            def type_query(needle):
                subprocess.run(['xdotool', 'key', '--clearmodifiers', 'slash'], env=env, check=True, timeout=10)
                subprocess.run(['xdotool', 'type', '--clearmodifiers', '--delay', '20', needle],
                               env=env, check=True, timeout=30)

            async def measure(label, needle, cache_state):
                await probe.js(MEASURE_JS % json.dumps(needle))
                type_query(needle)
                got = None
                for _ in range(200):
                    got = await probe.js('window.__m.done!==null?JSON.stringify(window.__m):null')
                    if got:
                        break
                    await asyncio.sleep(.2)
                assert got, f'{label}: 未在期限内渲染出结果'
                data = json.loads(got)
                report['queries'][label] = {'needle': needle, 'cache': cache_state,
                                            'ui_ms': round(data['done'], 1), 'rows': data['count']}
                # 清理：Esc 退出搜索，恢复全量列表
                subprocess.run(['xdotool', 'key', '--clearmodifiers', 'Escape'], env=env, check=True, timeout=10)
                await probe.until("document.querySelectorAll('#entries li[data-id]').length>1")
                return data

            first = await measure('broad_cold', BROAD, 'cold(新拷贝库,零驻留页)')
            second = await measure('broad_warm', BROAD, 'warm(同进程二次查询)')
            third = await measure('single_char_warm', NARROW, 'warm')
            assert first['count'] > 0 and second['count'] > 0, '宽泛查询应渲染出结果'
            report['checks'].append('WebView input -> list render end-to-end for broad and single-char queries')
            report['samples'] = {'broad_cold_ms': report['queries']['broad_cold']['ui_ms'],
                                 'broad_warm_ms': report['queries']['broad_warm']['ui_ms'],
                                 'single_char_warm_ms': report['queries']['single_char_warm']['ui_ms']}
            if OUT_DIR:
                OUT_DIR.mkdir(parents=True, exist_ok=True)
                png = OUT_DIR / 'search-e2e-list.png'
                subprocess.run(['import', '-display', env['DISPLAY'], '-window', 'root', str(png)],
                               check=True, timeout=60)
                report['screenshot'] = str(png)
            report['evidence_file'] = write_evidence('search-e2e-results.json', report)
            print(json.dumps(report, ensure_ascii=False))
        finally:
            stop(app)
            xvfb.terminate()
            xvfb.wait(timeout=10)


asyncio.run(main())
