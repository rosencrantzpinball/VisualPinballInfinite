#!/usr/bin/env python3
"""VPI against the owner's VPX fork on the Steam Frame, with the headset worn.

tools/steam-frame/versus.sh puts this on the Frame and starts it there:

    python3 versus.py <out folder> <plan.json>

Each run of the plan starts one of the two players on a table, waits until the headset is worn (the player draws,
and the head the test layer logs moves: SteamVR may draw for a headset lying still with its sensor covered), lets it
settle, measures, stops it, then starts the next. The tables are copies in the test folder: the fork writes a
cache folder, a picture and the table's saved values beside the table it plays. The fork keeps its settings, log and
library in a folder of the run (-PrefPath, a copy of the owner's settings), VPI in another (VPI_DATA).

Measured each second: the GPU time of the player's own work (the kernel's count for each GPU client, drm-engine-gpu in
/proc/<pid>/fdinfo, taken the same way for both players) and the compositor's, the GPU's clock, the temperatures, and
what each player reads of the runtime's GPU time counter (VPI: frames.csv; the fork: its status line every 5 s, with
VPX_XR_METRICS=1). Writes <out>/<run>/samples.csv, <out>/summary.json after each run, and <out>/done at the end.
Stops at once, and starts nothing more, when a game of the owner's starts.
"""

import json
import os
import re
import signal
import statistics
import subprocess
import sys
import time

HOME = os.path.expanduser('~')
HERE = os.path.dirname(os.path.abspath(__file__))
VPI = f'{HOME}/devkit-game/VisualPinballInfinite'
FORK = f'{HOME}/devkit-game/VisualPinball'
FORK_SETTINGS = f'{HOME}/.local/share/VPinballX/10.8/VPinballX.ini'
# the OpenXR test layer (xr-test-layer/xr_test_layer.c), sent beside this script
LAYER = os.path.join(HERE, 'xr-test-layer')
# Both players are started as Steam starts them (its logs: reaper SteamLaunch -- <game>/run.sh), on SteamOS itself,
# not in the Steam Linux Runtime's container, with the session's own environment; the fork also opens a desktop window
# (gamescope's X server)
SESSION = {'XDG_RUNTIME_DIR': '/run/user/1000', 'DBUS_SESSION_BUS_ADDRESS': 'unix:path=/run/user/1000/bus'}
APP = {'vpi': 'vpi-player', 'fork': 'VPinballX_BGFX'}
TEMPS = {'gpuss-0-thermal': 'gpu_c', 'cpuss0-thermal': 'cpu_c'}
# what of the owner's runs: a game Steam launched, VPX, a player not ours (as frame.sh's idle check)
OWNER = re.compile(r'SteamLaunch|VPinballX|vpinball|vpi-player')


def processes():
    """pid -> (parent, name) of every process."""
    out = {}
    for d in os.listdir('/proc'):
        if d.isdigit():
            try:
                with open(f'/proc/{d}/stat') as f:
                    s = f.read()
            except OSError:
                continue
            out[int(d)] = (int(s[s.rindex(')') + 2:].split()[1]), s[s.index('(') + 1:s.rindex(')')])
    return out


def tree(root, procs):
    """The process and all it started."""
    children = {}
    for pid, (parent, _) in procs.items():
        children.setdefault(parent, []).append(pid)
    found, todo = {root}, [root]
    while todo:
        for c in children.get(todo.pop(), []):
            if c not in found:
                found.add(c)
                todo.append(c)
    return found


def find(name, pids, procs):
    return next((p for p in sorted(pids) if procs.get(p, (0, ''))[1] == name), None)


def gpu_ns(pid):
    """The GPU time of a process's work so far, each DRM client counted once; None when it can't be read."""
    if pid is None:
        return None
    try:
        fds = os.listdir(f'/proc/{pid}/fdinfo')
    except OSError:
        return None
    seen, total = set(), 0
    for fd in fds:
        try:
            with open(f'/proc/{pid}/fdinfo/{fd}') as f:
                info = f.read()
        except OSError:
            continue
        client = re.search(r'^drm-client-id:\s*(\d+)', info, re.M)
        busy = re.search(r'^drm-engine-gpu:\s*(\d+)', info, re.M)
        if client and busy and client.group(1) not in seen:
            seen.add(client.group(1))
            total += int(busy.group(1))
    return total


def owner_busy(roots):
    """The owner's games running: command lines that look like one, not started by a run of ours (a player's
    helper starting as it loads is one of ours)."""
    procs = processes()

    def ours(pid):
        while pid > 1:
            if pid in roots or pid == os.getpid():
                return True
            pid = procs.get(pid, (0, ''))[0]
        return False

    busy = []
    for d in os.listdir('/proc'):
        if d.isdigit():
            try:
                with open(f'/proc/{d}/cmdline', 'rb') as f:
                    cmd = f.read().replace(b'\0', b' ').decode('utf-8', 'replace').strip()
            except OSError:
                continue
            if cmd and OWNER.search(cmd) and 'vpi-test' not in cmd and not ours(int(d)):
                busy.append(f'{d} {cmd[:200]}')
    return busy


def read_number(path, scale=1.0):
    try:
        with open(path) as f:
            return float(f.read().split()[0]) * scale
    except (OSError, ValueError, IndexError):
        return None


def temperatures():
    out = {}
    try:
        zones = os.listdir('/sys/class/thermal')
    except OSError:
        return out
    for z in zones:
        try:
            with open(f'/sys/class/thermal/{z}/type') as f:
                kind = f.read().strip()
        except OSError:
            continue
        if kind in TEMPS:
            out[TEMPS[kind]] = read_number(f'/sys/class/thermal/{z}/temp', 0.001)
    return out


def head_moved(lines):
    """Whether the head the layer logged in `lines` (its frames of the last second) moved: a worn headset always
    does, a little; one lying somewhere with its sensor covered is still, though SteamVR has it drawn."""
    poses = []
    for line in lines:
        c = line.split(',')
        if len(c) >= 16 and c[0].isdigit():
            try:
                poses.append([float(x) for x in c[11:16]])
            except ValueError:
                pass
    poses = [q for q in poses if all(v == v for v in q)]
    if len(poses) < 2:
        return False
    spread = [max(q[i] for q in poses) - min(q[i] for q in poses) for i in range(5)]
    # 0.3 degrees of pitch or yaw, or 3 mm, in a second
    return max(spread[0], spread[1]) > 0.3 or max(spread[2:]) > 0.003


class Lines:
    """The complete lines a file gained since the last read."""

    def __init__(self, path):
        self.path, self.pos, self.rest = path, 0, ''

    def read(self):
        try:
            with open(self.path, encoding='utf-8', errors='replace') as f:
                f.seek(self.pos)
                data = f.read()
                self.pos = f.tell()
        except OSError:
            return []
        lines = (self.rest + data).split('\n')
        self.rest = lines.pop()
        return lines


def set_ini(text, section, key, value):
    """`key = value` in `section` of a VPX settings file, the rest as it was."""
    nl = '\r\n' if '\r\n' in text else '\n'
    lines = text.splitlines(keepends=True)
    start = next((i for i, l in enumerate(lines) if l.strip().lstrip('﻿').lower() == f'[{section.lower()}]'), None)
    if start is None:
        return text + ('' if text.endswith(nl) or not text else nl) + f'[{section}]{nl}{key} = {value}{nl}'
    end = next((i for i in range(start + 1, len(lines)) if lines[i].lstrip().startswith('[')), len(lines))
    for i in range(start + 1, end):
        m = re.match(rf'\s*({re.escape(key)})\s*=', lines[i], re.I)
        if m:
            lines[i] = f'{m.group(1)} = {value}{nl}'
            return ''.join(lines)
    lines.insert(start + 1, f'{key} = {value}{nl}')
    return ''.join(lines)


def launch(r, d):
    """Starts a run's player in its own process group; returns the process and the files to follow."""
    settings = [(k.split('.', 1)[0], k.split('.', 1)[1], v) for k, v in r.get('settings', {}).items()]
    env = dict(os.environ, **SESSION)
    if r['player'] == 'vpi':
        data = os.path.join(d, 'data')
        os.makedirs(data)
        with open(os.path.join(data, 'Settings.ini'), 'w') as f:
            for section, key, value in settings:
                f.write(f'[{section}]\n{key} = {value}\n')
        env['VPI_DATA'] = data
        cwd, frames = VPI, os.path.join(d, 'frames.csv')
        cmd = ['./run.sh', r['table'], '--vr-timings', frames] + r.get('args', [])
        follow = {'frames': Lines(frames), 'log': Lines(os.path.join(data, 'vpi-player.log'))}
    else:
        pref = os.path.join(d, 'pref')
        os.makedirs(pref)
        with open(FORK_SETTINGS, encoding='utf-8', errors='surrogateescape') as f:
            text = f.read()
        for section, key, value in settings:
            text = set_ini(text, section, key, value)
        with open(os.path.join(pref, 'VPinballX.ini'), 'w', encoding='utf-8', errors='surrogateescape') as f:
            f.write(text)
        env.update(DISPLAY=':0', VPX_XR_METRICS='1')
        cwd = FORK
        cmd = [os.path.join(HERE, 'fork-run.sh'), '-Play', r['table'], '-PrefPath', pref] + r.get('args', [])
        follow = {'log': Lines(os.path.join(pref, 'vpinball.log'))}
    env.update(r.get('env', {}))
    if 'layer' in r:
        # each frame timed by the layer, the same way for both players; pinned: drawn unworn, the same view for both
        env.update(XR_API_LAYER_PATH=LAYER, XR_ENABLE_API_LAYERS='XR_APILAYER_VPI_test', VPI_XR_LAYER_LOG=os.path.join(d, 'xr-frames.csv'))
        follow['head'] = Lines(os.path.join(d, 'xr-frames.csv'))
        if r['layer'].get('pin'):
            env['VPI_XR_LAYER_PIN'] = '1'
    with open(os.path.join(d, 'command.txt'), 'w') as f:
        f.write(' '.join(f'{k}={v}' for k, v in env.items() if k.startswith(('XR_', 'VPI_', 'VPX_', 'DISPLAY'))) + ' ' + ' '.join(cmd) + '\n')
    out = open(os.path.join(d, 'launch.log'), 'w')
    p = subprocess.Popen(cmd, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT, start_new_session=True)
    return p, follow


def gone(p, seconds):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if p.poll() is not None:
            return True
        time.sleep(0.25)
    return p.poll() is not None


def stop(p, player):
    """Asks the player to close (VPI: SIGTERM, through SDL's quit; the fork: SIGINT, its own handler, which ends the
    session cleanly; a second SIGINT after 3 s makes it leave at once), then ends what is left of the run's group."""
    how = []
    procs = processes()
    app = find(APP[player], tree(p.pid, procs), procs)
    if app is not None and p.poll() is None:
        try:
            os.kill(app, signal.SIGINT if player == 'fork' else signal.SIGTERM)
            how.append('SIGINT' if player == 'fork' else 'SIGTERM')
        except ProcessLookupError:
            pass
        if not gone(p, 20) and player == 'fork':
            try:
                os.kill(app, signal.SIGINT)
                how.append('SIGINT again')
            except ProcessLookupError:
                pass
            gone(p, 8)
    for sig in (signal.SIGTERM, signal.SIGKILL):
        if p.poll() is None:
            try:
                os.killpg(p.pid, sig)
                how.append(f'group {sig.name}')
            except ProcessLookupError:
                pass
            gone(p, 5)
    return ', '.join(how) or 'had ended'


def run_one(r, d, deadline, stop_file, ours):
    """One run, sampled each second; returns its summary."""
    os.makedirs(d)
    p, follow = launch(r, d)
    ours.add(p.pid)
    started = time.monotonic()
    phase, since, result, fork_gpu, fork_at, states, window = 'wait', started, None, None, None, [], [None, None]
    last = {'app': None, 'comp': None, 't': started}
    rows = []
    samples = open(os.path.join(d, 'samples.csv'), 'w')
    samples.write('t,phase,app_gpu_ms,comp_gpu_ms,gpu_mhz,gpu_c,cpu_c,frames,drawn,vpi_gpu_ms,fork_gpu_ms\n')
    tick = started
    while result is None:
        tick += 1.0
        time.sleep(max(0.0, tick - time.monotonic()))
        now = time.monotonic()
        procs = processes()
        mine = tree(p.pid, procs)
        ours.update(mine)
        app = find(APP[r['player']], mine, procs)
        comp = find('vrcompositor', procs.keys(), procs)
        app_ns, comp_ns = gpu_ns(app), gpu_ns(comp)
        dt = now - last['t']
        app_ms = (app_ns - last['app']) / 1e6 / dt if app_ns is not None and last['app'] is not None and app_ns >= last['app'] else None
        comp_ms = (comp_ns - last['comp']) / 1e6 / dt if comp_ns is not None and last['comp'] is not None and comp_ns >= last['comp'] else None
        last.update(app=app_ns, comp=comp_ns, t=now)
        frames, drawn, vpi_gpu = 0, 0, []
        if r['player'] == 'vpi':
            for line in follow['frames'].read():
                c = line.split(',')
                if len(c) >= 9 and c[0].isdigit():
                    frames += 1
                    drawn += c[7] == '1'
                    if c[8]:
                        vpi_gpu.append(float(c[8]))
            for line in follow['log'].read():
                if line.startswith('VR: the session is'):
                    states.append((round(now - started, 1), line.rsplit(' ', 1)[-1]))
            rendering = drawn >= 20 and any(g > 1.0 for g in vpi_gpu)
        else:
            for line in follow['log'].read():
                if 'LogRuntimeStatus' in line:
                    m = re.search(r'gpu_frametime ([\d.]+) ms', line)
                    if m:
                        fork_gpu, fork_at = float(m.group(1)), now
                        states.append((round(now - started, 1), fork_gpu))
            rendering = fork_gpu is not None and fork_gpu > 1.0 and now - fork_at < 6.5
        moved = head_moved(follow['head'].read()) if 'head' in follow else None
        if moved is not None and not r['layer'].get('pin') and not moved:
            rendering = False
        temps = temperatures()
        row = {
            't': round(now - started, 2), 'phase': phase, 'app_gpu_ms': app_ms, 'comp_gpu_ms': comp_ms,
            'gpu_mhz': read_number('/sys/class/devfreq/3d00000.gpu/cur_freq', 1e-6), **temps, 'frames': frames, 'drawn': drawn,
            'vpi_gpu_ms': statistics.median(vpi_gpu) if vpi_gpu else None, 'vpi_gpu_all': vpi_gpu,
            'fork_gpu_ms': fork_gpu if r['player'] == 'fork' and fork_at == now else None, 'head_moved': moved,
        }
        rows.append(row)
        samples.write(','.join('' if row.get(k) is None else (f'{row[k]:.3f}' if isinstance(row[k], float) else str(row[k]))
                               for k in ('t', 'phase', 'app_gpu_ms', 'comp_gpu_ms', 'gpu_mhz', 'gpu_c', 'cpu_c', 'frames', 'drawn',
                                         'vpi_gpu_ms', 'fork_gpu_ms')) + '\n')
        samples.flush()
        if phase == 'wait' and rendering:
            phase, since = 'warm', now
        elif phase == 'warm' and now - since >= r['warm']:
            phase, since = 'measure', now
            window = [now, None]
        elif phase == 'measure' and now - since >= r['measure']:
            result = 'measured'
            window[1] = now
        if result is None:
            if p.poll() is not None:
                result = f'ended by itself ({p.returncode})'
            elif phase == 'wait' and now - since > r['wait']:
                result = 'nothing drawn (headset not worn?)'
            elif now > deadline:
                result = 'deadline'
            elif os.path.exists(stop_file):
                result = 'stopped (versus.sh stop)'
            elif busy := owner_busy(ours):
                result = 'owner'
                states.append(('owner', busy))
    if window[0] is not None and window[1] is None:
        window[1] = time.monotonic()
    ended = stop(p, r['player'])
    samples.close()
    s = summarise(r, rows, result, ended, states, started)
    if window[0] is not None and os.path.exists(os.path.join(d, 'xr-frames.csv')):
        s['pacing'] = pacing(os.path.join(d, 'xr-frames.csv'), window[0], window[1])
    return s


def summarise(r, rows, result, ended, states, started):
    window = [x for x in rows if x['phase'] == 'measure']

    def mean(k):
        v = [x[k] for x in window if x.get(k) is not None]
        return round(statistics.fmean(v), 3) if v else None

    s = {'name': r['name'], 'player': r['player'], 'table': os.path.basename(r['table']), 'settings': r.get('settings', {}),
         'result': result, 'stopped': ended, 'seconds': round(rows[-1]['t'], 1) if rows else 0, 'measured_s': len(window),
         'app_gpu_ms_per_s': mean('app_gpu_ms'), 'comp_gpu_ms_per_s': mean('comp_gpu_ms'), 'gpu_mhz': mean('gpu_mhz'),
         'gpu_c': [window[0].get('gpu_c'), window[-1].get('gpu_c')] if window else None, 'states': states,
         'head_moved_s': sum(1 for x in window if x.get('head_moved'))}
    if r['player'] == 'vpi':
        gpu = sorted(g for x in window for g in x['vpi_gpu_all'])
        fps = statistics.fmean(x['drawn'] for x in window) if window else 0
        s.update(fps=round(fps, 1), runtime_gpu_ms_median=round(statistics.median(gpu), 2) if gpu else None,
                 runtime_gpu_ms_p90=round(gpu[len(gpu) * 9 // 10], 2) if gpu else None,
                 app_gpu_ms_per_frame=round(s['app_gpu_ms_per_s'] / fps, 2) if fps and s['app_gpu_ms_per_s'] else None)
    else:
        gpu = [x['fork_gpu_ms'] for x in window if x['fork_gpu_ms'] is not None]
        s.update(runtime_gpu_ms=gpu, runtime_gpu_ms_median=round(statistics.median(gpu), 2) if gpu else None)
        if gpu and s['app_gpu_ms_per_s']:
            s['fps_estimate'] = round(s['app_gpu_ms_per_s'] / statistics.median(gpu), 1)
    return s


def pacing(path, start, end):
    """The frames the layer saw end between `start` and `end` (monotonic seconds): their rate, the display slots
    missed (a frame meant for more than one period after the one before), each frame's time from its wait's return
    to its end, and the runtime's GPU time."""
    rows = []
    with open(path) as f:
        next(f, None)
        for line in f:
            c = line.strip().split(',')
            if len(c) >= 11 and start * 1e3 <= float(c[4]) <= end * 1e3:
                rows.append([float(x) for x in c])
    if len(rows) < 2:
        return None
    period = statistics.median(c[7] for c in rows if c[7] > 0)
    gaps = [round((b[6] - a[6]) / period) - 1 for a, b in zip(rows, rows[1:])]
    seconds = (rows[-1][4] - rows[0][4]) / 1e3
    work = sorted(c[4] - c[1] - c[2] for c in rows if c[1] >= 0)
    gpu = sorted(c[10] for c in rows if c[10] > 0)

    def q(v, f):
        return round(v[min(len(v) - 1, int(len(v) * f))], 2) if v else None

    return {'frames': len(rows), 'seconds': round(seconds, 1), 'fps': round((len(rows) - 1) / seconds, 1),
            'period_ms': round(period, 2), 'slots_missed': sum(g for g in gaps if g > 0),
            'hitches': sum(1 for g in gaps if g > 0), 'hitches_per_min': round(sum(1 for g in gaps if g > 0) / seconds * 60, 1),
            'work_ms': [q(work, 0.5), q(work, 0.9), q(work, 0.99), q(work, 1.0)],
            'gpu_ms': [q(gpu, 0.5), q(gpu, 0.9), q(gpu, 0.99), q(gpu, 1.0)],
            'gpu_over_period': sum(1 for g in gpu if g > period)}


def main():
    out, plan = sys.argv[1], json.load(open(sys.argv[2]))
    os.makedirs(out)
    deadline = time.monotonic() + plan.get('deadline', 1800)
    ours, summary = set(), []
    busy = owner_busy(ours)
    if busy:
        summary.append({'result': 'not started: the owner is playing', 'processes': busy})
    else:
        for r in plan['runs']:
            s = run_one(r, os.path.join(out, r['name']), deadline, os.path.join(out, 'stop'), ours)
            summary.append(s)
            with open(os.path.join(out, 'summary.json'), 'w') as f:
                json.dump(summary, f, indent=1)
            print(json.dumps({k: v for k, v in s.items() if k != 'states'}), flush=True)
            if s['result'] in ('owner', 'deadline') or s['result'].startswith('stopped') \
                    or (s['result'].startswith('nothing drawn') and plan.get('stop_when_not_worn', True)):
                break
            time.sleep(plan.get('gap', 4))
    with open(os.path.join(out, 'summary.json'), 'w') as f:
        json.dump(summary, f, indent=1)
    open(os.path.join(out, 'done'), 'w').close()


if __name__ == '__main__':
    main()
