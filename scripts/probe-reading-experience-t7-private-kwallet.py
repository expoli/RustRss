"""Bounded KWallet/Secret Service probe under a caller-provided private bus.

Use the same mktemp HOME/XDG_RUNTIME_DIR + dbus-run-session wrapper documented
by verify-reading-experience-t7-linux-actions.py. This script never touches the
host session bus, host HOME or a user wallet and never answers unlock prompts.
"""
import asyncio
import json
import os
from pathlib import Path
import re
import subprocess
import sys


async def main():
    out = Path(sys.argv[1]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    root = Path(os.environ['HOME']).parent
    assert str(root).startswith('/tmp/rustrss-t7-linux-')
    assert os.environ['XDG_CONFIG_HOME'] == str(root/'config')
    assert os.environ['XDG_DATA_HOME'] == str(root/'data')
    assert os.environ.get('DBUS_SESSION_BUS_ADDRESS')
    assert not os.environ.get('DISPLAY') and not os.environ.get('WAYLAND_DISPLAY')
    assert not os.environ.get('RUSTSS_AI_KEY')
    display = next(':'+str(i) for i in range(120,140) if not Path(f'/tmp/.X11-unix/X{i}').exists())
    env = dict(os.environ,DISPLAY=display,QT_QPA_PLATFORM='xcb',GDK_BACKEND='x11',GDK_GL='disable',GTK_USE_PORTAL='0')
    xvfb_path = os.environ.get('T7_XVFB','/tmp/rustrss-t4-xvfb/root/usr/bin/Xvfb')
    probe_path = os.environ.get('T7_KEYRING_PROBE','/home/tcy/Github/RustRss/target/debug/examples/keyring_probe')
    assert Path(xvfb_path).is_file() and Path(probe_path).is_file()
    config = root/'config'/'kwalletrc'
    config.write_text('[Wallet]\nEnabled=true\nFirst Use=false\nDefault Wallet=kdewallet\nLocal Wallet=localwallet\nUse One Wallet=true\n')
    report = {'display':display,'private_root':str(root),'commands':[], 'result':'blocked'}
    xvfb = subprocess.Popen([xvfb_path,display,'-screen','0','1280x800x24','-nolisten','tcp','-ac'],env=env,
                            stdout=(out/'private-kwallet-xvfb.log').open('w'),stderr=subprocess.STDOUT)
    kwallet = None
    secrets = None
    try:
        for _ in range(100):
            assert xvfb.poll() is None
            if Path('/tmp/.X11-unix/X'+display[1:]).exists():break
            await asyncio.sleep(.1)
        assert Path('/tmp/.X11-unix/X'+display[1:]).exists()
        subprocess.run(['dbus-update-activation-environment','DISPLAY','QT_QPA_PLATFORM'],env=env,check=True,capture_output=True,text=True)
        kwallet = subprocess.Popen(['/usr/bin/kwalletd6'],env=env,
                                  stdout=(out/'private-kwalletd6.log').open('w'),stderr=subprocess.STDOUT)
        secrets = subprocess.Popen(['/usr/bin/ksecretd'],env=env,
                                  stdout=(out/'private-ksecretd.log').open('w'),stderr=subprocess.STDOUT)
        await asyncio.sleep(.5)
        cmd = [probe_path]
        try:
            completed = subprocess.run(cmd,env=env,capture_output=True,text=True,timeout=8)
            report['commands'].append({'argv':cmd,'exit_code':completed.returncode,'stdout':completed.stdout,'stderr':completed.stderr})
            report['result'] = 'pass' if completed.returncode == 0 else 'blocked'
        except subprocess.TimeoutExpired as error:
            report['commands'].append({'argv':cmd,'timeout_seconds':8,'error':repr(error)})
            report['result'] = 'blocked-interactive-or-hung'
        for cmd in [
            ['gdbus','call','--session','--dest','org.freedesktop.secrets','--object-path','/org/freedesktop/secrets','--method','org.freedesktop.Secret.Service.ReadAlias','default'],
            ['gdbus','call','--session','--dest','org.freedesktop.secrets','--object-path','/org/freedesktop/secrets','--method','org.freedesktop.DBus.Properties.Get','org.freedesktop.Secret.Service','Collections'],
            ['xwininfo','-root','-tree'],
        ]:
            try:
                completed = subprocess.run(cmd,env=env,capture_output=True,text=True,timeout=5)
                report['commands'].append({'argv':cmd,'exit_code':completed.returncode,'stdout':completed.stdout,'stderr':completed.stderr})
            except subprocess.TimeoutExpired as error:
                report['commands'].append({'argv':cmd,'timeout_seconds':5,'error':repr(error)})
        create = ['gdbus','call','--session','--dest','org.freedesktop.secrets',
                  '--object-path','/org/freedesktop/secrets','--method','org.freedesktop.Secret.Service.CreateCollection',
                  "{'org.freedesktop.Secret.Collection.Label': <'T7 private collection'>}",'default']
        try:
            completed = subprocess.run(create,env=env,capture_output=True,text=True,timeout=5)
            report['commands'].append({'argv':create,'exit_code':completed.returncode,'stdout':completed.stdout,'stderr':completed.stderr})
            paths = re.findall(r"objectpath '([^']+)'",completed.stdout)
            if completed.returncode == 0 and len(paths)>=2 and paths[0]!='/' and paths[1]=='/':
                # A collection was created without an interactive Prompt.
                retried = subprocess.run([probe_path],env=env,capture_output=True,text=True,timeout=8)
                report['commands'].append({'argv':[probe_path],'exit_code':retried.returncode,
                                           'stdout':retried.stdout,'stderr':retried.stderr})
                if retried.returncode == 0:report['result']='pass'
            elif completed.returncode == 0:
                report['result']='blocked-interactive-collection-prompt'
        except subprocess.TimeoutExpired as error:
            report['commands'].append({'argv':create,'timeout_seconds':5,'error':repr(error)})
            report['result']='blocked-interactive-or-hung'
        report['daemons']={'kwalletd6_alive':kwallet.poll() is None,'ksecretd_alive':secrets.poll() is None}
    finally:
        for proc in [secrets,kwallet,xvfb]:
            if proc and proc.poll() is None:
                proc.terminate()
                try:proc.wait(timeout=5)
                except subprocess.TimeoutExpired:proc.kill();proc.wait(timeout=5)
        (out/'private-kwallet-results.json').write_text(json.dumps(report,ensure_ascii=False,indent=2)+'\n')
        print(json.dumps({'result':report['result'],'out':str(out)},ensure_ascii=False),flush=True)


if __name__ == '__main__':
    asyncio.run(main())
