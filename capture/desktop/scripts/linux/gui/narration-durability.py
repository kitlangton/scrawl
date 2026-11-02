#!/usr/bin/env python3
""":"
exec python3 "$0" "$@"
":"""
DESCRIPTION = """Guest-only #257 scenario; no VM controller, installs, downloads or timed recorder.

Run via gui-vm.py --audio-file <synthetic PCM16 WAV, 8..60s> --isolated-network.
The sanitized guest needs pactl, pw-play, parecord, grim, ffmpeg, libX11 and
normal capture/portal dependencies. Parent must approve each native portal
chooser within 60s. SCRAWL_GUI_BINARY must be the staged raw ELF (not an
AppImage launcher): exact executable/argv/PID ownership is required for signals.
SCRAWL_GUI_OUTPUT is supplied by gui-vm.py. SCRAWL_GUI_ARTIFACTS optionally
selects a NEW subdirectory of that output; defaults to output/artifacts.

Uses a unique module-null-sink, never physical speakers or system defaults.
Config selects microphone.id=pulse:<owned sink>.monitor, autoPasteDraft=false,
launchAtLogin=false. HOME/config/cache/data are private; desktop bus/runtime
remain real, as in capture-smoke.py. No sibling scenario imports: the runner
stages just this shell/Python polyglot. Total budget 480s including cleanup.

PASS means observed recipe checks only. Full video/pixel review and exact
callback-to-disk crash tail remain UNRUN. Independent source WAV duration is
reported, NOT mislabelled as callback chronology or a bound on lost samples.
"""

import argparse
import array
import ctypes as C
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time
import uuid
import wave

EXECUTION_SECONDS = 420
CLEANUP_SECONDS = 45


class Failure(Exception):
    pass


def pcm(path):
    """Measure actual decoded PCM, not file size or a claimed wall-clock length."""
    with wave.open(str(path), 'rb') as source:
        if source.getsampwidth() != 2 or source.getcomptype() != 'NONE':
            raise Failure(f'{path}: require uncompressed PCM16 WAV')
        channels, rate, declared = source.getnchannels(), source.getframerate(), source.getnframes()
        samples = array.array('h', source.readframes(declared))
    if sys.byteorder != 'little':
        samples.byteswap()
    frames = len(samples) // channels
    if frames != declared or not samples or not rate:
        raise Failure(f'{path}: empty or truncated WAV')
    return {'frames': frames, 'channels': channels, 'rate': rate,
            'duration_seconds': frames / rate,
            'peak': max(abs(s) for s in samples),
            'rms': (sum(s * s for s in samples) / len(samples)) ** .5}


def require_audible(path):
    measured = pcm(path)
    if measured['peak'] < 100 or measured['rms'] < 20:
        raise Failure(f'{path}: no independently measurable synthetic audio: {measured}')
    return measured


def receipt(value, owned, status):
    if value.get('capture_id') != owned or value.get('status') != status or value.get('error') is not None:
        raise Failure(f'expected exact owned {status} receipt: {value}')
    path = value.get('bundle_path')
    if status == 'saved':
        if not isinstance(path, str) or not Path(path).is_absolute():
            raise Failure('saved receipt lacks absolute path')
        return Path(path)
    if path is not None:
        raise Failure('non-saved receipt contains bundle path')


def scene():
    """Same libX11 synthetic changing-window mechanism as capture-smoke.py."""
    x = C.CDLL('libX11.so.6')
    p, u, i = C.c_void_p, C.c_ulong, C.c_int
    signatures = {
        'XOpenDisplay': (p, [C.c_char_p]), 'XDefaultRootWindow': (u, [p]),
        'XCreateSimpleWindow': (u, [p, u, i, i, C.c_uint, C.c_uint, C.c_uint, u, u]),
        'XStoreName': (i, [p, u, C.c_char_p]), 'XMapWindow': (i, [p, u]),
        'XCreateGC': (p, [p, u, u, p]), 'XSetForeground': (i, [p, p, u]),
        'XFillRectangle': (i, [p, u, p, i, i, C.c_uint, C.c_uint]),
        'XDrawString': (i, [p, u, p, i, i, C.c_char_p, i]), 'XSync': (i, [p, i]),
    }
    for name, (result, args) in signatures.items():
        getattr(x, name).restype, getattr(x, name).argtypes = result, args
    display = x.XOpenDisplay(None)
    if not display:
        raise Failure('cannot open synthetic scene DISPLAY')
    window = x.XCreateSimpleWindow(display, x.XDefaultRootWindow(display), 60, 100, 800, 500, 0, 0, 0x17335B)
    x.XStoreName(display, window, b'Apparent narration durability - SYNTHETIC')
    gc = x.XCreateGC(display, window, 0, None)
    x.XMapWindow(display, window)
    for tick in range(570):
        x.XSetForeground(display, gc, 0x17335B if tick % 2 else 0x8B2750)
        x.XFillRectangle(display, window, gc, 0, 0, 800, 500)
        x.XSetForeground(display, gc, 0xFFFFFF)
        text = f'SYNTHETIC NARRATION DURABILITY | {tick:04d} | no private data'.encode()
        x.XDrawString(display, window, gc, 40, 120, text, len(text))
        x.XSync(display, 0)
        time.sleep(.5)


class Scenario:
    def __init__(self, output, artifacts, binary, stimulus):
        self.output, self.out, self.binary, self.stimulus = output, artifacts, binary, stimulus
        self.env = os.environ.copy()
        self.jobs, self.ids, self.module, self.app = [], [], None, None
        self.end = time.monotonic() + EXECUTION_SECONDS  # leave cleanup time within a 480s runner
        self.serial = 0
        self.report = {'schema': 1, 'result': 'RUNNING', 'commands': [], 'captures': {},
                       'cleanup_errors': [], 'gates': {key: 'UNRUN' for key in (
                           'environment', 'independent_input', 'normal', 'cancel', 'crash',
                           'sigterm', 'cleanup', 'artifacts', 'exact_crash_tail', 'visual_review', 'runner_cleanup')},
                       'limits': ['No callback source count: exact crash-tail bound unavailable.',
                                  'Independent monitor duration is not the app callback sample count.',
                                  'Parent must inspect screenshots, sheet and complete normal movie.']}

    def save(self):
        (self.out / 'narration-durability.json').write_text(json.dumps(self.report, indent=2) + '\n')

    def remaining(self, seconds):
        left = self.end - time.monotonic()
        if left <= 0:
            raise Failure('420s execution budget exhausted; remaining stages UNRUN')
        return min(seconds, left)

    def call(self, argv, seconds=8, check=True):
        self.serial += 1
        stem = f'command-{self.serial:03d}'
        record = {'argv': list(map(str, argv)), 'stdout': stem + '.stdout', 'stderr': stem + '.stderr'}
        self.report['commands'].append(record)
        self.save()
        with (self.out / record['stdout']).open('w') as stdout, (self.out / record['stderr']).open('w') as stderr:
            result = subprocess.run(record['argv'], env=self.env, stdout=stdout, stderr=stderr,
                                    timeout=self.remaining(seconds))
        record['exit'] = result.returncode
        self.save()
        if check and result.returncode:
            raise Failure(f'command failed ({result.returncode}); see {stem}: {argv}')
        return (self.out / record['stdout']).read_text()

    def control(self, *args, seconds=8, check=True):
        value = json.loads(self.call([self.binary, 'capture', *args], seconds, check))
        if not isinstance(value, dict):
            raise Failure('invalid GUI control JSON')
        if check and value.get('error'):
            raise Failure(f'GUI command failed: {value}')
        return value

    def launch(self, argv, name):
        with (self.out / (name + '.log')).open('w') as log:
            job = subprocess.Popen(list(map(str, argv)), env=self.env, stdout=log,
                                   stderr=subprocess.STDOUT, start_new_session=True)
        self.jobs.append(job)
        return job

    def wait(self, predicate, seconds, message):
        deadline = time.monotonic() + self.remaining(seconds)
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(.2)
        raise Failure(message)

    def signal_exact(self, job, sig, argv=None):
        # No numeric PID signals after wait/poll reaps the ownership anchor.
        if job.returncode is not None:
            raise Failure('cannot signal a reaped child')
        exited = os.waitid(os.P_PID, job.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
        if exited is not None:
            raise Failure(f'owned child {job.pid} exited before requested signal')
        if argv is not None:
            actual = (Path('/proc') / str(job.pid) / 'cmdline').read_bytes().rstrip(b'\0').split(b'\0')
            if actual != [os.fsencode(a) for a in argv] or not os.path.samefile(f'/proc/{job.pid}/exe', self.binary):
                raise Failure(f'owned child {job.pid} is not the exact raw candidate app command')
        os.kill(job.pid, sig)

    def stop_child(self, job, sig=signal.SIGTERM):
        if job.returncode is None:
            if os.waitid(os.P_PID, job.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is None:
                self.signal_exact(job, sig)
            self.reap_owned(job, 4)

    def reap_owned(self, job, seconds):
        # Keep the child unreaped until its session descendants are stopped.
        # An unreaped leader pins its numeric group ID against PID reuse.
        self.wait(lambda: os.waitid(os.P_PID, job.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None,
                  seconds, f'owned child {job.pid} did not exit')
        try:
            os.killpg(job.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        def group_dead():
            for proc in Path('/proc').iterdir():
                if not proc.name.isdigit():
                    continue
                try:
                    fields = (proc / 'stat').read_text().rsplit(')', 1)[1].split()
                except (FileNotFoundError, ProcessLookupError):
                    continue
                if int(fields[2]) == job.pid and fields[0] not in {'Z', 'X'}:
                    return False
            return True
        self.wait(group_dead, 2, f'owned process group {job.pid} still live; retain guest')
        return job.wait(timeout=self.remaining(1))

    def screenshot(self, name):
        # Compositor pixels, not the recorder's candidate images.
        self.call(['grim', str(self.out / (name + '.png'))])

    def prepare(self):
        desktop = self.call(['systemctl', '--user', 'show-environment'])
        for line in desktop.splitlines():
            key, sep, value = line.partition('=')
            if sep and key in {'DISPLAY', 'WAYLAND_DISPLAY', 'XDG_RUNTIME_DIR', 'DBUS_SESSION_BUS_ADDRESS',
                               'HYPRLAND_INSTANCE_SIGNATURE', 'XAUTHORITY', 'XDG_CURRENT_DESKTOP', 'XDG_SESSION_TYPE'}:
                self.env[key] = value
        if not self.env.get('DISPLAY') or not self.env.get('WAYLAND_DISPLAY') or not self.env.get('XDG_RUNTIME_DIR'):
            raise Failure('requires owned Wayland/Hyprland guest plus XWayland')
        if (Path(self.env['XDG_RUNTIME_DIR']) / 'apparent.sock').exists():
            raise Failure('existing apparent.sock; refuse shared/live desktop')
        for proc in Path('/proc').iterdir():
            try:
                if proc.name.isdigit() and proc.stat().st_uid == os.getuid() and (proc / 'comm').read_text().strip() in {'apparent', 'scrawl'}:
                    raise Failure('existing product process; use a clean isolated guest')
            except (FileNotFoundError, PermissionError, ProcessLookupError):
                pass
        authority = Path(self.env['HOME']) / '.Xauthority'
        if 'XAUTHORITY' not in self.env and authority.is_file():
            self.env['XAUTHORITY'] = str(authority)
        for name in ('pactl', 'pw-play', 'parecord', 'grim', 'ffmpeg'):
            if not shutil.which(name):
                raise Failure(f'missing guest prerequisite {name}; no installation attempted')
        with self.binary.open('rb') as candidate:
            if candidate.read(4) != b'\x7fELF':
                raise Failure('exact signal scenario requires staged raw ELF candidate')
        # AppImage ELF launchers still fail exact /proc argv/exe verification before signaling.
        for key, path in {'HOME': self.output, 'XDG_CONFIG_HOME': self.output / '.config',
                          'XDG_DATA_HOME': self.output / '.local/share', 'XDG_CACHE_HOME': self.output / '.cache',
                          'TMPDIR': self.output / 'tmp', 'APPARENT_CONFIG': self.output / '.config/apparent/config.json'}.items():
            self.env[key] = str(path)
        if len(os.fsencode(self.output / '.config/apparent/agent-control/capture.sock')) > 107:
            raise Failure('HOME too long for Unix control socket; runner must use shorter guest output')
        for key in ('XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_CACHE_HOME', 'TMPDIR'):
            Path(self.env[key]).mkdir(parents=True, exist_ok=False)
        self.report['stimulus'] = require_audible(self.stimulus)
        if not 8 <= self.report['stimulus']['duration_seconds'] <= 60:
            raise Failure('synthetic stimulus must be 8..60 seconds of PCM16')
        shutil.copyfile(self.stimulus, self.out / 'stimulus.wav')
        self.report['candidate_sha256'] = hashlib.sha256(self.binary.read_bytes()).hexdigest()
        self.sink = 'apparent_durability_' + uuid.uuid4().hex
        loaded = self.call(['pactl', 'load-module', 'module-null-sink', 'sink_name=' + self.sink,
                            'rate=16000', 'channels=1', 'sink_properties=device.description=SYNTHETIC_NARRATION'])
        self.module = int(loaded.strip())
        self.source = self.sink + '.monitor'
        sources = json.loads(self.call(['pactl', '--format=json', 'list', 'sources']))
        if not any(s.get('name') == self.source for s in sources):
            raise Failure('owned null-sink monitor not present')
        self.report['routing'] = {'sink': self.sink, 'source': self.source, 'module': self.module,
                                  'physical_output': False, 'system_defaults_changed': False}
        config = Path(self.env['APPARENT_CONFIG'])
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(json.dumps({'onboarding': {'seen': True}, 'launchAtLogin': False,
                                      'autoPasteDraft': False,
                                      'microphone': {'id': 'pulse:' + self.source, 'name': 'Synthetic narration monitor'}}))
        self.report['config'] = json.loads(config.read_text())
        self.gate('environment')

    def gate(self, name):
        self.report['gates'][name] = 'PASS'
        self.save()

    def stimulus_start(self, label):
        monitor = self.launch(['parecord', '--device=' + self.source, '--file-format=wav',
                               '--format=s16le', '--rate=16000', '--channels=1',
                               str(self.out / (label + '-source.wav'))], label + '-monitor')
        time.sleep(.4)
        player = self.launch(['pw-play', '--target=' + self.sink,
                              '--properties={ node.dont-fallback = true node.dont-reconnect = true }',
                              str(self.stimulus)], label + '-player')
        return monitor, player

    def stimulus_stop(self, label, jobs):
        monitor, player = jobs
        self.stop_child(player)
        self.stop_child(monitor, signal.SIGINT)  # parecord closes/finalizes the WAV header
        measured = require_audible(self.out / (label + '-source.wav'))
        if measured['duration_seconds'] < 2:
            raise Failure(f'{label}: independent source has less than two seconds of PCM')
        self.report.setdefault('source_measurements', {})[label] = measured
        self.save()
        return measured

    def launch_app(self, label):
        self.app = self.launch([self.binary, 'app'], label + '-app')
        app = self.app
        def ready():
            if os.waitid(os.P_PID, app.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None:
                raise Failure('GUI candidate exited before control readiness')
            value = self.control('state', check=False)
            return not value.get('error') and value.get('state') == {'phase': 'idle', 'capture_id': None}
        self.wait(ready, 15, 'GUI did not become idle/ready')

    def remove_sockets(self):
        # Only private known endpoints after exact owned GUI exit; never desktop runtime.
        for relative in ('.config/apparent/agent-control/capture.sock', '.config/apparent/launch/open.sock'):
            path = self.output / relative
            if path.is_socket():
                path.unlink()

    def start(self, label):
        captures = self.out / ('captures-' + label)
        captures.mkdir()
        # Each GUI process is launched with a distinct capture root; normal and cancel
        # also get separate processes, to keep directory attribution unambiguous.
        self.env['APPARENT_CAPTURES_DIR'] = str(captures)
        self.launch_app(label)
        assert self.app is not None
        owned = uuid.uuid4().hex
        self.ids.append((self.app, owned))
        self.report['captures'][label] = {'capture_id': owned, 'app_pid': self.app.pid,
                                          'app_argv': [str(self.binary), 'app'], 'root': str(captures)}
        self.save()  # identity durable BEFORE Start
        print(f'{label}: approve only this guest synthetic output in portal within 60s', flush=True)
        receipt(self.control('start', '--request-id', owned, '--wait', '--timeout', '60', seconds=64), owned, 'recording')
        if self.control('state').get('state') != {'phase': 'recording', 'capture_id': owned}:
            raise Failure('owned recording receipt/state mismatch')
        def active_bundle():
            return len(list(captures.glob('*.apparent/.audio-recording'))) == 1
        self.wait(active_bundle, 4, 'missing unique recording-time narration journal')
        bundle = next(captures.glob('*.apparent/.audio-recording')).parent
        self.report['captures'][label]['active_bundle'] = str(bundle.relative_to(self.output))
        self.save()
        return owned, bundle

    def native_audio(self, label, bundle):
        measured = require_audible(bundle / 'audio.wav')
        if measured['rate'] != 16000 or measured['channels'] != 1 or measured['duration_seconds'] < 2:
            raise Failure(f'{label}: missing meaningful 16kHz mono native audio: {measured}')
        self.report['captures'][label]['native_audio'] = measured
        self.save()

    def exit_app(self):
        assert self.app is not None
        self.signal_exact(self.app, signal.SIGTERM, [self.binary, 'app'])
        self.reap_owned(self.app, 12)
        self.remove_sockets()

    def run(self):
        self.report['stage'] = 'environment'
        self.prepare()
        self.report['stage'] = 'independent_input'
        jobs = self.stimulus_start('preflight')
        time.sleep(3)
        self.stimulus_stop('preflight', jobs)
        self.gate('independent_input')  # must pass BEFORE any app audio claim
        self.scene_job = self.launch([sys.executable, Path(__file__).resolve(), '--scene'], 'scene')
        time.sleep(.7)
        self.screenshot('synthetic-scene')
        for label in ('normal', 'cancel', 'crash', 'sigterm'):
            self.report['stage'] = label
            self.save()
            owned, bundle = self.start(label)
            assert self.app is not None
            jobs = self.stimulus_start(label)
            time.sleep(5)
            self.screenshot(label + '-recording')
            if label == 'normal':
                saved = receipt(self.control('stop', owned, '--wait', '--timeout', '35', seconds=39), owned, 'saved')
                assert saved is not None
                if saved.resolve() != bundle.resolve() or receipt(self.control('inspect', owned), owned, 'saved') != saved:
                    raise Failure('saved path differs from exact owned active bundle')
                for name in ('summary.md', 'contact-sheet.png', 'apparent.json'):
                    if not (saved / name).is_file() or not (saved / name).stat().st_size:
                        raise Failure(f'normal bundle lacks {name}')
                json.loads((saved / 'apparent.json').read_text())
                self.native_audio(label, saved)
                self.screenshot('normal-saved')
                self.exit_app()
            elif label == 'cancel':
                receipt(self.control('cancel', owned), owned, 'cancelled')
                receipt(self.control('inspect', owned), owned, 'cancelled')
                current = self.control('state').get('state', {})
                if current.get('capture_id') is not None or current.get('phase') not in {'idle', 'completed'}:
                    raise Failure('cancel left active state')
                self.exit_app()
                root = bundle.parent
                self.call([self.binary, 'recover', '--captures', str(root)], seconds=12)
                # Check disk AFTER recovery, not merely a cancelled receipt.
                residue = [str(p) for p in root.rglob('*') if p.name in {
                    'audio.wav', '.audio-recording', 'narration-recovered.txt', 'apparent.json'}]
                if residue:
                    raise Failure(f'cancel retained recoverable audio/bundle: {residue}')
            elif label == 'crash':
                if not (bundle / '.unsafe-video').is_file() or not (bundle / '.audio-recording').is_file():
                    raise Failure('active crash target lacks unsafe-video/narration markers')
                self.report['captures'][label]['signal'] = 'SIGKILL exact owned GUI child PID'
                self.signal_exact(self.app, signal.SIGKILL, [self.binary, 'app'])
                if self.reap_owned(self.app, 4) != -signal.SIGKILL:
                    raise Failure('crash child did not exit by SIGKILL')
                self.remove_sockets()
                self.stimulus_stop(label, jobs)
                jobs = None
                # Exercise default settle/pending recovery, not explicit-bundle bypass.
                time.sleep(16)
                self.call([self.binary, 'recover', '--captures', str(bundle.parent)], seconds=15)
                if not (bundle / 'narration-recovered.txt').is_file() or (bundle / '.audio-recording').exists():
                    raise Failure('dead-writer settled narration was not recovered')
                if not (bundle / '.unsafe-video').exists() or any((bundle / p).exists() for p in (
                        'apparent.json', 'summary.md', 'contact-sheet.png', 'frames', 'frames-full')):
                    raise Failure('unsafe crash video was promoted into screen handoff evidence')
                self.native_audio(label, bundle)
                self.report['captures'][label]['lost_tail_samples'] = None
                self.report['captures'][label]['lost_tail_bound'] = 'UNAVAILABLE: no app callback source count'
            else:
                self.signal_exact(self.app, signal.SIGTERM, [self.binary, 'app'])
                code = self.reap_owned(self.app, 35)
                self.report['captures'][label]['exit_code'] = code
                if code != 0:
                    raise Failure(f'SIGTERM did not exit gracefully: {code}')
                self.remove_sockets()
                if (bundle / '.audio-recording').exists() or not (bundle / 'apparent.json').is_file():
                    raise Failure('SIGTERM failed to finalize active capture before exit')
                self.native_audio(label, bundle)
            if jobs:
                self.stimulus_stop(label, jobs)
            self.gate(label)
        self.report['result'] = 'PASS_RECIPE_GATES_ONLY'

    def cleanup(self):
        # No pkill, guessed PID, global cancel or reaped-child group signal.
        self.end = time.monotonic() + 20
        for app, owned in self.ids:
            if app.returncode is None:
                try:
                    if os.waitid(os.P_PID, app.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is None:
                        self.control('cancel', owned, seconds=2, check=False)
                except Exception as error:
                    self.report['cleanup_errors'].append(str(error))
        for job in reversed(self.jobs):
            if job.returncode is not None:
                continue
            try:
                self.stop_child(job)
            except Exception as error:
                self.report['cleanup_errors'].append(str(error))
                try:
                    self.signal_exact(job, signal.SIGKILL)
                    self.reap_owned(job, 1)
                except Exception as failure:
                    self.report['cleanup_errors'].append(str(failure))
        try:
            if not self.report['cleanup_errors']:
                self.remove_sockets()
            if self.module is not None:
                self.call(['pactl', 'unload-module', str(self.module)], seconds=2)
        except Exception as error:
            self.report['cleanup_errors'].append(str(error))
        self.report['gates']['cleanup'] = 'FAIL' if self.report['cleanup_errors'] else 'PASS'
        self.save()

    def retain(self):
        # Real screenshots -> actual sheet. No placeholder PNGs or invented bundle metadata.
        try:
            # Do not include an existing contact sheet in its own input glob.
            if not list(self.out.glob('*-recording.png')):
                raise Failure('no native recording screenshots; contact sheet UNRUN')
            self.call(['ffmpeg', '-v', 'error', '-pattern_type', 'glob', '-i', str(self.out / '*.png'),
                       '-vf', 'scale=480:-1,tile=3x3', '-frames:v', '1', '-y',
                       str(self.out / 'contact-sheet.png')], seconds=4)
        except Exception as error:
            self.report['contact_sheet_error'] = str(error)
            if self.report['result'] == 'PASS_RECIPE_GATES_ONLY':
                self.report['result'] = 'FAIL'
            self.report['gates']['artifacts'] = 'FAIL'
        else:
            self.report['gates']['artifacts'] = 'PASS'
        self.save()
        lines = ['# Synthetic narration durability', '', f"Result: {self.report['result']}", '',
                 'Exact crash-tail bound: unavailable (no app callback source count).',
                 'All audio is synthetic and routed only through an owned null sink.', '']
        lines += [f'- {name}: {value}' for name, value in self.report['gates'].items()]
        if self.report.get('failure'):
            lines += ['', self.report['failure']]
        (self.out / 'summary.md').write_text('\n'.join(lines) + '\n')
        # Entire private output, including native WAVs/bundles, not just the report folder.
        manifest = {}
        for path in sorted(self.output.rglob('*')):
            if path.is_symlink():
                continue
            if path.is_file() and path != self.out / 'manifest.json':
                manifest[str(path.relative_to(self.output))] = hashlib.sha256(path.read_bytes()).hexdigest()
        (self.out / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=DESCRIPTION)
    parser.add_argument('--scene', action='store_true', help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.scene:
        scene()
        return 0
    os.umask(0o077)
    output = Path(os.environ['SCRAWL_GUI_OUTPUT']).resolve()
    artifacts = Path(os.environ.get('SCRAWL_GUI_ARTIFACTS', str(output / 'artifacts'))).resolve()
    if artifacts == output or not artifacts.is_relative_to(output):
        parser.error('SCRAWL_GUI_ARTIFACTS must be a new child of runner output for retention')
    if (output / '.config').exists() or artifacts.exists():
        parser.error('output already used; preserve evidence and choose a new run')
    artifacts.mkdir(parents=True)
    scenario = Scenario(output, artifacts, Path(os.environ['SCRAWL_GUI_BINARY']).resolve(),
                        Path(os.environ['SCRAWL_GUI_AUDIO_FILE']).resolve())
    def interrupted(*_):
        raise Failure('scenario interrupted; incomplete gates remain UNRUN')
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGALRM, interrupted)
    signal.alarm(EXECUTION_SECONDS + 5)
    try:
        scenario.run()
    except (Failure, OSError, ValueError, EOFError, subprocess.SubprocessError, wave.Error) as error:
        scenario.report['result'] = 'FAIL'
        scenario.report['failure'] = str(error)
        stage = scenario.report.get('stage')
        if stage in scenario.report['gates']:
            scenario.report['gates'][stage] = 'FAIL'
        print(str(error), file=sys.stderr)
    finally:
        # Hard backstop even if a cleanup primitive wedges. A killed cleanup is
        # not certified: persist its pending gate before arming that backstop.
        scenario.save()
        signal.signal(signal.SIGALRM, lambda *_: os._exit(124))
        signal.alarm(CLEANUP_SECONDS)
        scenario.cleanup()
        scenario.retain()
        signal.alarm(0)
    return 0 if scenario.report['result'] == 'PASS_RECIPE_GATES_ONLY' and not scenario.report['cleanup_errors'] else 1


if __name__ == '__main__':
    sys.exit(main())
