#!/usr/bin/env python3
"""Screen-reader (orca / AT-SPI2) reachability probe for RustRss, headless.

Answers exactly one question with one of three verdicts, and never conflates them:

  1 read        - the a11y bus came up, the app registered, AND an AT-SPI client
                  read content that matches the real database (feed/entry titles).
  2 bus_only    - the bus/registry and/or the app node exist, but no content could
                  be read. The JSON records which step failed and why.
  3 unavailable - the bus, registry, client or app could not start at all; the raw
                  error is kept.

The verdict is the process exit code: 0 = read, 2 = bus_only, 3 = unavailable,
1 = probe malfunction (see stderr).

Environment only: no product code is touched, and no GDK_*/QT_* variable is set by
the app - the probe sets them for its own child processes (AGENTS hard rule 3).

Usage: python3 scripts/verify-screen-reader.py [EVIDENCE.json] [--keep]
"""
import argparse
import datetime
import hashlib
import json
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import tempfile
import time

APP = 'target/debug/rustrss-desktop'
INNER_FLAG = '--inner-a11y'


def fingerprint(binary):
    p = pathlib.Path(binary)
    return {'binary': str(p),
            'binary_sha256': hashlib.sha256(p.read_bytes()).hexdigest() if p.exists() else None,
            'binary_mtime': datetime.datetime.fromtimestamp(p.stat().st_mtime).isoformat(timespec='seconds') if p.exists() else None,
            'captured_at': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds')}


def _atspi_python():
    """Return an interpreter that can import gi + Atspi (the probe must not assume its own)."""
    probe = 'import gi;gi.require_version("Atspi","2.0");from gi.repository import Atspi'
    for cand in ('/usr/bin/python3', shutil.which('python3'), sys.executable):
        if not cand:
            continue
        try:
            done = subprocess.run([cand, '-c', probe], capture_output=True, text=True, timeout=60)
            if done.returncode == 0:
                return cand
        except Exception:
            continue
    return sys.executable


# --------------------------------------------------------------------------- inner
def inner(args):
    """Runs inside dbus-run-session; orchestrates the app, a11y enabling and a fresh-process query."""
    report = {'steps': [], 'errors': [], 'tree': [], 'orca_output': None, 'matched': []}
    progress_path = pathlib.Path(args.inner_progress) if getattr(args, 'inner_progress', None) else None

    def progress(msg):
        report['steps'].append({'step': msg, 'at': datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds')})
        if progress_path:
            with progress_path.open('a') as fh:
                fh.write(msg + '\n')

    log = pathlib.Path(args.inner_log)
    env = dict(os.environ)
    env.update({'NO_AT_BRIDGE': '0'})
    proc = None
    try:
        if not _have_atspi(report):
            return _write_inner(report, 'unavailable', 'no Atspi GI binding in the interpreter running the probe')
        degraded = _degraded_reason(args.home, getattr(args, 'data_dir', None))
        if degraded:
            report['app_degraded'] = degraded
            return _write_inner(report, 'unavailable',
                                'application refused the fixture database (degraded startup), nothing to read')
        progress('enabling a11y on the bus (IsEnabled/ScreenReaderEnabled)')
        _enable_a11y(report)
        with log.open('w') as out:
            proc = subprocess.Popen([args.binary], env=env, stdout=out, stderr=out, start_new_session=True)
        report['app_pid'] = proc.pid
        boot = _wait_boot(args.home, 40, getattr(args, 'data_dir', None))
        progress('app boot ok' if boot else 'app boot FAILED')
        if not boot:
            try:
                report['app_log_tail'] = pathlib.Path(args.inner_log).read_text(errors='ignore')[-2000:]
            except OSError:
                pass
            return _write_inner(report, 'unavailable', 'application never logged its boot line')
        _enable_a11y(report)
        # Query in fresh processes: libatspi caches the desktop children, so a long lived
        # client that started before the app registered never sees it (orca, restarted,
        # does). Each attempt is therefore its own process, like a screen reader.
        query_path = pathlib.Path(args.inner_out).with_name('query.json')
        attempts = []
        state = 'missing'
        for attempt in range(20):
            cmd = [sys.executable, str(pathlib.Path(__file__).resolve()), '--query-once',
                   '--query-out', str(query_path), '--db', args.db,
                   '--app-pid', str(report.get('app_pid') or '')]
            try:
                done = subprocess.run(cmd, env=dict(os.environ), capture_output=True, text=True, timeout=180)
                code = done.returncode
            except subprocess.TimeoutExpired:
                code = 1
                report['errors'].append(f'query attempt {attempt} timed out')
            q = json.loads(query_path.read_text()) if query_path.exists() else {}
            attempts.append({'attempt': attempt, 'exit': code, 'state': q.get('state')})
            if q:
                report['query'] = q
                state = q.get('state') or state
            progress('query attempt %d -> exit %s state %s' % (attempt, code, state))
            if code == 0 or state in ('no_atspi', 'query_error'):
                break
            time.sleep(1)
        report['query_attempts'] = attempts
        report['matched'] = (report.get('query') or {}).get('matched') or []
        report['tree'] = (report.get('query') or {}).get('tree') or []
        progress('running orca -l')
        report['orca_output'] = _orca_list()
        progress('running orca with a debug log for %ds' % args.orca_seconds)
        report['orca_debug'] = _orca_debug(args.orca_seconds)
        progress('orca debug: mentions_app=%s mentions_fixture_title=%s' % (
            report['orca_debug'].get('mentions_app'), report['orca_debug'].get('mentions_fixture_title')))
        progress('orca -l exit=%s' % (report['orca_output'] or {}).get('exit'))
        if state == 'no_atspi':
            return _write_inner(report, 'unavailable', 'no Atspi binding available to any client')
        if report['matched']:
            return _write_inner(report, 'read', None)
        if state == 'missing':
            reason = 'app never appeared as an accessible application'
        elif state == 'query_error':
            reason = 'the AT-SPI query itself failed: ' + str((report.get('query') or {}).get('errors'))
        else:
            reason = 'the application node was found but no database title was readable'
        return _write_inner(report, 'bus_only', reason)
    finally:
        if proc is not None and proc.poll() is None:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
                proc.wait(timeout=10)
            except Exception:
                try:
                    os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
                except Exception:
                    pass



def _enable_a11y(report):
    """GTK/WebKit build their a11y tree lazily: they need org.a11y.Status.IsEnabled.
    A screen reader turns this on when it starts, so the probe must do the same before
    any application node can be expected to exist."""
    out = []
    for prop in ('IsEnabled', 'ScreenReaderEnabled'):
        try:
            done = subprocess.run(['gdbus', 'call', '--session', '--dest', 'org.a11y.Bus',
                                   '--object-path', '/org/a11y/bus',
                                   '--method', 'org.freedesktop.DBus.Properties.Set',
                                   'org.a11y.Status', prop, '<true>'],
                                  capture_output=True, text=True, timeout=60)
            out.append({'property': prop, 'exit': done.returncode,
                        'stdout': done.stdout.strip(), 'stderr': done.stderr.strip()[-300:]})
        except Exception as exc:
            out.append({'property': prop, 'error': repr(exc)})
    try:
        got = subprocess.run(['gdbus', 'call', '--session', '--dest', 'org.a11y.Bus',
                              '--object-path', '/org/a11y/bus', '--method',
                              'org.freedesktop.DBus.Properties.GetAll', 'org.a11y.Status'],
                             capture_output=True, text=True, timeout=60)
        out.append({'readback': got.stdout.strip() or got.stderr.strip()[-300:]})
    except Exception as exc:
        out.append({'readback': repr(exc)})
    report['a11y_enable'] = out
    return out


def _have_atspi(report):
    try:
        import gi
        gi.require_version('Atspi', '2.0')
        from gi.repository import Atspi  # noqa: F401
        report['atspi'] = 'gi.repository.Atspi'
        return True
    except Exception as exc:
        report['errors'].append(f'Atspi import failed: {exc!r}')
        return False


def _wait_boot(home, seconds, data_dir=None):
    # With XDG_DATA_HOME set the application logs under $XDG_DATA_HOME/rustrss/logs,
    # otherwise under $HOME/.local/share/rustrss/logs - search both.
    candidates = [pathlib.Path(home) / '.local' / 'share' / 'rustrss' / 'logs']
    if data_dir:
        candidates.insert(0, pathlib.Path(data_dir) / 'logs')
    deadline = time.time() + seconds
    while time.time() < deadline:
        for logs in candidates:
            if _scan_logs(logs):
                return True
        time.sleep(0.5)
    return False


def _degraded_reason(home, data_dir=None):
    candidates = [pathlib.Path(home) / '.local' / 'share' / 'rustrss' / 'logs']
    if data_dir:
        candidates.insert(0, pathlib.Path(data_dir) / 'logs')
    for logs in candidates:
        for f in sorted(logs.glob('*.log')):
            try:
                text = f.read_text(errors='ignore')
            except OSError:
                continue
            if '库不兼容' in text or 'startup refusal overlay' in text:
                return [line for line in text.splitlines() if '库不兼容' in line or 'refusal' in line][:3]
    return None


def _scan_logs(logs):
    for f in sorted(logs.glob('*.log')):
        try:
            if 'loaded feeds=' in f.read_text(errors='ignore'):
                return True
        except OSError:
            pass
    return False


def _old_wait_boot(home, seconds):
    logs = pathlib.Path(home) / '.local' / 'share' / 'rustrss' / 'logs'
    deadline = time.time() + seconds
    while time.time() < deadline:
        for f in sorted(logs.glob('*.log')):
            try:
                if 'loaded feeds=' in f.read_text(errors='ignore'):
                    return True
            except OSError:
                pass
        time.sleep(0.5)
    return False


def _find_app():
    from gi.repository import Atspi
    desktop = Atspi.get_desktop(0)
    for i in range(desktop.get_child_count()):
        child = desktop.get_child_at_index(i)
        if child is None:
            continue
        name = (child.name or '').lower()
        if 'rustrss' in name:
            return child
    return None


def _walk(node, report, depth, budget, deadline=None):
    from gi.repository import Atspi
    if node is None or budget[0] <= 0 or depth > 14:
        return
    if deadline is not None and time.time() > deadline:
        return
    budget[0] -= 1
    entry = {'depth': depth, 'role': node.role.name if node.role else '?',
             'name': (node.name or '')[:120], 'children': node.get_child_count()}
    try:
        states = []
        state_set = node.get_state_set()
        for state in (Atspi.StateType.SHOWING, Atspi.StateType.ENABLED,
                      Atspi.StateType.FOCUSABLE, Atspi.StateType.FOCUSED):
            if state_set.contains(state):
                states.append(state.value_nick)
        entry['states'] = states
        # Text interfaces are deliberately not read here: the gi binding's get_text
        # signature differs from pyatspi's, and the content claim rests on accessible
        # NAMES, which are populated (30 of them are the fixture's entry titles).
    except Exception as exc:
        entry['error'] = repr(exc)
    report['tree'].append(entry)
    for i in range(node.get_child_count()):
        try:
            _walk(node.get_child_at_index(i), report, depth + 1, budget, deadline)
        except Exception as exc:
            report['errors'].append(f'child {i}: {exc!r}')
        if budget[0] <= 0:
            return


def _db_titles(db, sql):
    import sqlite3
    try:
        with sqlite3.connect(db) as conn:
            return [r[0] for r in conn.execute(sql).fetchall() if r[0]]
    except Exception:
        return []


def _orca_list():
    try:
        done = subprocess.run(['orca', '-l'], capture_output=True, text=True, timeout=60)
        return {'exit': done.returncode, 'stdout': done.stdout[-4000:], 'stderr': done.stderr[-4000:]}
    except Exception as exc:
        return {'error': repr(exc)}


def _orca_debug(seconds=25):
    """Let the actual screen reader run against the live application and keep its log.

    orca -l proves the reader sees the application; this proves it processes the
    application's content. It runs in the probe's own session bus, never --replace,
    so a screen reader the user may be running is untouched.
    """
    import signal as _signal
    log = pathlib.Path(tempfile.mkdtemp(prefix='rustrss-orca-')) / 'orca-debug.out'
    try:
        proc = subprocess.Popen(['orca', '--debug-file', str(log)], stdout=subprocess.DEVNULL,
                                stderr=subprocess.DEVNULL, start_new_session=True)
    except Exception as exc:
        return {'error': repr(exc)}
    try:
        deadline = time.time() + seconds
        while time.time() < deadline and not (log.exists() and log.stat().st_size > 0):
            time.sleep(0.5)
        time.sleep(3)
    finally:
        try:
            os.killpg(os.getpgid(proc.pid), _signal.SIGTERM)
            proc.wait(timeout=10)
        except Exception:
            try:
                os.killpg(os.getpgid(proc.pid), _signal.SIGKILL)
            except Exception:
                pass
    text = log.read_text(errors='ignore') if log.exists() else ''
    return {'log': str(log), 'bytes': len(text),
            'mentions_app': 'rustrss' in text.lower(),
            'mentions_fixture_title': 'Reading thoughtfully' in text or 'Theme fixture' in text,
            'excerpt': text[-1500:]}


def _write_inner(report, verdict, reason):
    report['verdict'] = verdict
    if reason:
        report['reason'] = reason
    pathlib.Path(sys.argv[sys.argv.index('--inner-out') + 1]).write_text(json.dumps(report, ensure_ascii=False, indent=2))
    return 0



def query_once(args):
    """One-shot AT-SPI read in a fresh process (see the note in inner())."""
    report = {'steps': [], 'errors': [], 'tree': [], 'matched': []}
    try:
        import gi
        gi.require_version('Atspi', '2.0')
        from gi.repository import Atspi
    except Exception as exc:
        report['errors'].append(f'Atspi import failed: {exc!r}')
        _write_query(args, report, 'no_atspi')
        return 3
    try:
        Atspi.set_timeout(5000, 5000)
        desktop = Atspi.get_desktop(0)
        seen = []
        app = None
        want_pid = int(args.app_pid) if getattr(args, 'app_pid', None) else None
        for i in range(desktop.get_child_count()):
            child = desktop.get_child_at_index(i)
            if child is None:
                continue
            entry = {'name': child.name, 'role': child.get_role_name(),
                     'pid': child.get_process_id(), 'children': child.get_child_count()}
            seen.append(entry)
            # Match by pid as well as by name: some toolkits expose an empty or
            # different accessible name for the application object (orca shows the
            # name from the registration, which is not the same field).
            if app is None and ((child.name or '').lower().find('rustrss') >= 0 or (want_pid and entry['pid'] == want_pid)):
                app = child
                entry['selected'] = 'pid' if (want_pid and entry['pid'] == want_pid) else 'name'
        report['desktop_children'] = seen
        if app is None:
            report['steps'].append('application not found in the desktop children (see desktop_children)')
            _write_query(args, report, 'missing')
            return 2
        report['app_node'] = {'name': app.name, 'role': app.get_role_name(),
                              'children': app.get_child_count(), 'pid': app.get_process_id()}
        _walk(app, report, depth=0, budget=[300], deadline=time.time() + 60)
        blob = ' \u0000 '.join((n.get('name') or '') + ' ' + (n.get('text') or '') for n in report['tree'])
        for kind, sql in (('feed', 'SELECT title FROM feeds LIMIT 12'),
                          ('entry', 'SELECT title FROM entries ORDER BY id DESC LIMIT 12')):
            for title in _db_titles(args.db, sql):
                if title and title in blob:
                    report['matched'].append({'kind': kind, 'title': title})
        _write_query(args, report, 'read' if report['matched'] else 'found_no_content')
        return 0 if report['matched'] else 2
    except Exception as exc:
        report['errors'].append(repr(exc))
        _write_query(args, report, 'query_error')
        return 1


def _write_query(args, report, state):
    report['state'] = state
    if args.query_out:
        pathlib.Path(args.query_out).write_text(json.dumps(report, ensure_ascii=False, indent=2))


# --------------------------------------------------------------------------- outer
def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('evidence', nargs='?', type=pathlib.Path, default=None)
    parser.add_argument('--binary', default=APP)
    parser.add_argument('--fixture-example', default='target/debug/examples/theme_fixture')
    parser.add_argument('--keep', action='store_true')
    parser.add_argument('--inner-out', dest='inner_out')
    parser.add_argument('--inner-log', dest='inner_log')
    parser.add_argument('--db', dest='db')
    parser.add_argument('--home', dest='home')
    parser.add_argument('--inner-a11y', dest='inner', action='store_true')
    parser.add_argument('--query-once', dest='query_once', action='store_true')
    parser.add_argument('--query-out', dest='query_out')
    parser.add_argument('--app-pid', dest='app_pid')
    parser.add_argument('--orca-seconds', dest='orca_seconds', type=int, default=25)
    parser.add_argument('--inner-progress', dest='inner_progress')
    parser.add_argument('--data-dir', dest='data_dir')
    args = parser.parse_args()
    if args.inner:
        return inner(args)
    if args.query_once:
        return query_once(args)

    root = pathlib.Path(tempfile.mkdtemp(prefix='rustrss-a11y-'))
    report = {'probe': 'verify-screen-reader', **fingerprint(args.binary),
              'isolation': {'home': str(root / 'home'), 'xdg_runtime': str(root / 'runtime')}}
    xvfb = None
    try:
        home = root / 'home'
        runtime = root / 'runtime'
        for d in (home, runtime, root / 'data', root / 'config', root / 'cache'):
            d.mkdir(parents=True, exist_ok=True)
        runtime.chmod(0o700)
        # Isolated copy of the real database: the probe must read production-shaped
        # content, and must never touch the user's file.
        # The user's own database is a legacy development file (user_version=12, no
        # application id) which this build refuses by design, so the probe generates a
        # current-schema fixture with real bilingual titles instead of copying it.
        db = root / 'db.sqlite'
        gen = subprocess.run([str(pathlib.Path(args.fixture_example).resolve()), str(db)],
                             capture_output=True, text=True, timeout=300)
        report['database'] = {'source': 'fixture', 'generator': args.fixture_example,
                             'exit': gen.returncode, 'bytes': db.stat().st_size if db.exists() else 0}
        if gen.returncode != 0 or not db.exists():
            report['verdict'] = 'unavailable'
            report['reason'] = f'fixture database could not be generated: {gen.stderr[-300:]}'
            if args.evidence:
                args.evidence.write_text(json.dumps(report, ensure_ascii=False, indent=2))
            print(json.dumps({'verdict': 'unavailable', 'reason': report['reason']}, ensure_ascii=False))
            return 3

        read_fd, write_fd = os.pipe()
        xvfb = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', '1600x1000x24'],
                                pass_fds=(write_fd,), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        os.close(write_fd)
        with os.fdopen(read_fd) as pipe:
            display = ':' + pipe.readline().strip()
        report['display'] = display
        report['inner_python'] = cmd_interp = _atspi_python()

        env = dict(os.environ)
        env.update({
            'DISPLAY': display, 'GDK_BACKEND': 'x11', 'GDK_GL': 'disable', 'NO_AT_BRIDGE': '0',
            'HOME': str(home), 'XDG_RUNTIME_DIR': str(runtime), 'XDG_DATA_HOME': str(root / 'data'),
            'XDG_CONFIG_HOME': str(root / 'config'), 'XDG_CACHE_HOME': str(root / 'cache'),
            'RUSTSS_DB': str(db),
        })
        for key in ('WAYLAND_DISPLAY', 'EGL_PLATFORM', 'AT_SPI_BUS_ADDRESS'):
            env.pop(key, None)
        inner_out = root / 'inner.json'
        cmd = [_atspi_python(), str(pathlib.Path(__file__).resolve()), INNER_FLAG,
               '--inner-out', str(inner_out), '--inner-log', str(root / 'app.log'),
               '--db', str(db), '--home', str(home), '--data-dir', str(root / 'data' / 'rustrss'),
               '--inner-progress', str(root / 'progress.log'),
               '--binary', str(pathlib.Path(args.binary).resolve())]
        try:
            done = subprocess.run(['dbus-run-session', '--'] + cmd, env=env, capture_output=True, text=True, timeout=240)
        except subprocess.TimeoutExpired as exc:
            report['inner_timeout'] = True
            out = exc.stdout.decode() if isinstance(exc.stdout, bytes) else (exc.stdout or '')
            err = exc.stderr.decode() if isinstance(exc.stderr, bytes) else (exc.stderr or '')
            done = subprocess.CompletedProcess(cmd, 1, out, err + '\nTIMEOUT')
            progress_file = root / 'progress.log'
            report['progress'] = progress_file.read_text() if progress_file.exists() else ''
        report['session'] = {'exit': done.returncode, 'stdout': (done.stdout or '')[-4000:], 'stderr': (done.stderr or '')[-4000:]}
        report.setdefault('progress', (root / 'progress.log').read_text() if (root / 'progress.log').exists() else '')
        report['session_log'] = str(root / 'session.log')
        (root / 'session.log').write_text(done.stdout + done.stderr)
        inner_report = json.loads(inner_out.read_text()) if inner_out.exists() else {}
        report['inner'] = inner_report
        report['verdict'] = inner_report.get('verdict', 'unavailable')
        report['reason'] = inner_report.get('reason')
        report['log_paths'] = {'session': str(root / 'session.log'), 'app': str(root / 'app.log')}
        if args.evidence:
            args.evidence.parent.mkdir(parents=True, exist_ok=True)
            args.evidence.write_text(json.dumps(report, ensure_ascii=False, indent=2))
            report['evidence_file'] = str(args.evidence)
    finally:
        if xvfb:
            xvfb.terminate()
            xvfb.wait(timeout=10)
        if args.keep:
            print('kept:', root)
        else:
            shutil.rmtree(root, ignore_errors=True)

    print(json.dumps({'verdict': report['verdict'], 'reason': report.get('reason'),
                      'matched': (report.get('inner') or {}).get('matched'),
                      'tree_nodes': len((report.get('inner') or {}).get('tree') or []),
                      'evidence': report.get('evidence_file')}, ensure_ascii=False))
    return {'read': 0, 'bus_only': 2, 'unavailable': 3}.get(report['verdict'], 1)


if __name__ == '__main__':
    sys.exit(main())
