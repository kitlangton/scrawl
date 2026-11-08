#!/usr/bin/env python3
"""Click-test an exact AppImage in a disposable Omarchy desktop.

QMP sends real pointer/key input; OCR locates and asserts actual rendered text.
No GPUI model events, update commands, or runtime test hooks produce the results.
Private evidence includes every sampled frame, request log, statuses and movie.
"""
import argparse
import csv
import hashlib
import io
import json
from pathlib import Path
import re
import socket
import subprocess
import sys
import time


def tokens(text):
    return re.findall(r'[^\W_]+', text.casefold())


class PhraseMatchError(ValueError):
    def __init__(self, phrase, count):
        super().__init__(f'expected one visible {phrase!r}; found {count}')
        self.count = count


def erase_long_light_lines(image):
    """OCR-only preprocessing: keep text; remove bright straight button outlines."""
    image = image.convert('L')
    pixels = image.load()
    width, height = image.size
    lines = []
    for vertical in [False, True]:
        for outer in range(width if vertical else height):
            start = None
            limit = height if vertical else width
            for inner in range(limit + 1):
                bright = inner < limit and pixels[(outer, inner) if vertical else (inner, outer)] > 190
                if bright and start is None:
                    start = inner
                if not bright and start is not None:
                    if inner - start >= 30:
                        lines.extend((outer, i) if vertical else (i, outer) for i in range(start, inner))
                    start = None
    for x, y in lines:
        pixels[x, y] = min(pixels[max(0, x - 2), max(0, y - 2)],
                           pixels[min(width - 1, x + 2), min(height - 1, y + 2)])
    return image


def find_phrase(words, phrase):
    """Return the center of one unique rendered phrase; never guess a position."""
    lines = {}
    for word in words:
        if tokens(word['text']):
            key = tuple(word[k] for k in ('block_num', 'par_num', 'line_num'))
            lines.setdefault(key, []).append(word)
    wanted = tokens(phrase)
    matches = []
    runs = []
    for line in lines.values():
        run = []
        for word in sorted(line, key=lambda w: int(w['left'])):
            if run:
                previous = run[-1]
                gap = int(word['left']) - int(previous['left']) - int(previous['width'])
                if gap > max(24, 1.5 * min(int(previous['height']), int(word['height']))):
                    runs.append(run)
                    run = []
            run.append(word)
        if run:
            runs.append(run)
    for line in runs:
        if [token for word in line for token in tokens(word['text'])] != wanted:
            continue
        # Tesseract normally separates the labels into one word per row.
        for start in range(len(line)):
            selected = []
            actual = []
            for word in line[start:]:
                actual.extend(tokens(word['text']))
                selected.append(word)
                if actual == wanted:
                    x0 = min(int(w['left']) for w in selected)
                    y0 = min(int(w['top']) for w in selected)
                    x1 = max(int(w['left']) + int(w['width']) for w in selected)
                    y1 = max(int(w['top']) + int(w['height']) for w in selected)
                    matches.append(((x0 + x1) // 2, (y0 + y1) // 2))
                    break
                if actual != wanted[:len(actual)]:
                    break
    if len(matches) != 1:
        raise PhraseMatchError(phrase, len(matches))
    return matches[0]


def qmp(path, command, arguments):
    with socket.socket(socket.AF_UNIX) as sock:
        sock.settimeout(10)
        sock.connect(str(path))
        stream = sock.makefile('rwb')
        json.loads(stream.readline())
        for name, args in [('qmp_capabilities', {}), (command, arguments)]:
            stream.write((json.dumps({'execute': name, 'arguments': args}) + '\n').encode())
            stream.flush()
            while True:
                reply = json.loads(stream.readline())
                if 'error' in reply:
                    raise RuntimeError(reply)
                if 'return' in reply:
                    break
        return reply['return']


def candidate_matches(path, expected):
    try:
        return path.read_text().strip() == expected
    except OSError:
        return False


def dialog_dismissed(variants):
    for words in variants:
        for phrase in ['Update available', 'Update later', 'Update now']:
            try:
                find_phrase(words, phrase)
                return False
            except PhraseMatchError as error:
                if error.count:
                    return False
    for words in variants:
        try:
            find_phrase(words, 'Check for Updates')
            return True
        except PhraseMatchError as error:
            if error.count:
                return False
    return False


class Evidence:
    def __init__(self, directory):
        self.directory = directory
        self.frames = directory / 'frames'
        self.frames.mkdir()
        self.clock = time.monotonic()
        self.samples = []
        self.steps = []
        self.socket = directory / 'vm/qmp.sock'

    def frame(self):
        path = self.frames / f'{len(self.samples):06}.png'
        qmp(self.socket, 'screendump', {'filename': str(path), 'format': 'png'})
        stamp = time.monotonic() - self.clock
        self.samples.append({'file': str(path), 'seconds': stamp})
        return path

    def words(self, path, cleaned=False):
        if cleaned:
            from PIL import Image
            derived = self.directory / 'ocr-input.png'
            with Image.open(path) as image:
                erase_long_light_lines(image).save(derived)
            path = derived
        result = subprocess.run(['tesseract', str(path), 'stdout', '--psm', '11', 'tsv'],
                                check=True, capture_output=True, text=True, timeout=10)
        self.last_ocr = result.stdout
        return list(csv.DictReader(io.StringIO(result.stdout), delimiter='\t'))

    def locate(self, phrase, timeout):
        deadline = time.monotonic() + timeout
        last = None
        while True:
            path = self.frame()
            words = self.words(path)
            try:
                position = find_phrase(words, phrase)
                return path, position
            except ValueError as error:
                last = error
                if isinstance(error, PhraseMatchError) and error.count == 0:
                    # Derived OCR input removes outlines; raw evidence is intact.
                    words = self.words(path, cleaned=True)
                    try:
                        return path, find_phrase(words, phrase)
                    except ValueError as cleaned_error:
                        last = cleaned_error
            if time.monotonic() >= deadline:
                (self.directory / 'failed-ocr.tsv').write_text(self.last_ocr)
                raise RuntimeError(f'{last}; last frame: {path}')
            time.sleep(0.1)

    def perform(self, request):
        start = time.monotonic() - self.clock
        op = request['op']
        if op in ('click', 'assert'):
            path, (x, y) = self.locate(request['text'], request.get('timeout', 20))
            if op == 'click':
                from PIL import Image
                with Image.open(path) as image:
                    width, height = image.size
                qmp(self.socket, 'input-send-event', {'events': [
                    {'type': 'abs', 'data': {'axis': 'x', 'value': round(x * 32767 / (width - 1))}},
                    {'type': 'abs', 'data': {'axis': 'y', 'value': round(y * 32767 / (height - 1))}}]})
                # Let the compositor apply motion/focus before separate button
                # edges. A down/up batch can collapse before GPUI sees a click.
                time.sleep(0.15)
                for down in [True, False]:
                    qmp(self.socket, 'input-send-event', {'events': [
                        {'type': 'btn', 'data': {'down': down, 'button': 'left'}}]})
                    time.sleep(0.08)
                # Do not let the pointer obscure the small label in the next OCR
                # sample (especially the disabled Checking state).
                qmp(self.socket, 'input-send-event', {'events': [
                    {'type': 'abs', 'data': {'axis': 'x', 'value': 32700}},
                    {'type': 'abs', 'data': {'axis': 'y', 'value': 32700}}]})
        elif op == 'assert-dismissed':
            deadline = time.monotonic() + request.get('timeout', 20)
            while True:
                path = self.frame()
                if dialog_dismissed([self.words(path), self.words(path, cleaned=True)]):
                    break
                if time.monotonic() >= deadline:
                    raise RuntimeError(f'update dialog did not disappear; last frame: {path}')
                time.sleep(0.1)
        elif op == 'key':
            if request['key'] != 'esc':
                raise ValueError('only Escape is supported')
            for down in [True, False]:
                qmp(self.socket, 'input-send-event', {'events': [
                    {'type': 'key', 'data': {'down': down, 'key': {'type': 'qcode', 'data': 'esc'}}}]})
            path = self.frame()
        elif op == 'hold':
            path = self.frame()
            deadline = time.monotonic() + request['seconds']
            while time.monotonic() < deadline:
                path = self.frame()
                time.sleep(0.2)
        else:
            raise ValueError(f'unknown GUI operation {op}')
        self.steps.append(dict(request, start=start, end=time.monotonic() - self.clock, frame=str(path)))
        (self.directory / 'steps.json').write_text(json.dumps(self.steps, indent=2) + '\n')
        # A readable real-time hold also captures the click's resulting pixels.
        for _ in range(3 if op == 'assert' else 1):
            time.sleep(0.2)
            self.frame()

    def movie(self):
        if len(self.samples) < 2:
            return
        start = self.samples[0]['seconds']
        listing = ['ffconcat version 1.0']
        for index, sample in enumerate(self.samples[:-1]):
            duration = self.samples[index + 1]['seconds'] - sample['seconds']
            listing.extend([f"file '{sample['file']}'", f'duration {duration:.6f}'])
        listing.append(f"file '{self.samples[-1]['file']}'")
        concat = self.directory / 'frames.ffconcat'
        concat.write_text('\n'.join(listing) + '\n')
        subprocess.run(['ffmpeg', '-v', 'error', '-y', '-safe', '0', '-i', str(concat),
                        '-vf', 'fps=30', '-c:v', 'libx264', '-preset', 'fast', '-crf', '20',
                        '-pix_fmt', 'yuv420p', '-movflags', '+faststart', str(self.directory / 'raw.mp4')], check=True)
        intervals = [b['seconds'] - a['seconds'] for a, b in zip(self.samples, self.samples[1:])]
        (self.directory / 'recording.json').write_text(json.dumps({
            'method': 'QMP compositor samples, variable cadence; encoded 30 fps, NOT smooth 30 fps capture',
            'first_frame_clock': start, 'duration': self.samples[-1]['seconds'] - start,
            'max_sample_gap_seconds': max(intervals), 'samples': self.samples}, indent=2) + '\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['template', 'ovmf-vars', 'ssh-key', 'appimage', 'results']:
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--ssh-user', required=True)
    parser.add_argument('--channel', choices=['stable', 'main'], default='stable')
    args = parser.parse_args()
    args.results = args.results.resolve()
    if args.results.exists():
        parser.error('results must be a new private directory')
    with args.appimage.open('rb') as image:
        header = image.read(11)
    if header[:4] != b'\x7fELF' or header[8:11] != b'AI\x02':
        parser.error('candidate must be a real type-2 AppImage')
    args.results.mkdir(mode=0o700, parents=True)
    source = Path(__file__).resolve().parent
    repository = source.parents[4]
    source_head = subprocess.check_output(['git', '-C', str(repository), 'rev-parse', 'HEAD'], text=True).strip()
    if args.channel == 'main' and (not re.fullmatch('[0-9a-f]{40}', source_head) or source_head == 'b' * 40):
        parser.error('Main fixture needs a 40hex source HEAD distinct from its synthetic candidate')
    repository_dirty = subprocess.check_output(
        ['git', '-C', str(repository), 'status', '--porcelain', '--untracked-files=all'], text=True)
    if repository_dirty:
        parser.error('repository must be clean before testing a packaged candidate (including docs and tooling)')
    with args.appimage.open('rb') as image:
        digest = hashlib.file_digest(image, 'sha256').hexdigest()
    fixture = (source / 'updater-fixture.py').read_text()
    scenario = (source / 'updater-gui-scenario.sh').read_text().replace(
        '__UPDATER_FIXTURE_SOURCE__', "cat > \"$out/fixture.py\" <<'UPDATER_FIXTURE_EOF'\n" + fixture + '\nUPDATER_FIXTURE_EOF')
    scenario = scenario.replace('__UPDATER_EXPECTED_SOURCE_HEAD__', source_head)
    scenario = scenario.replace('__UPDATER_EXPECTED_SHA256__', digest)
    scenario = scenario.replace('__UPDATER_CHANNEL__', args.channel)
    scenario_path = args.results / 'scenario.sh'
    scenario_path.write_text(scenario)
    (args.results / 'candidate.json').write_text(json.dumps({
        'appimage': str(args.appimage.resolve()), 'sha256': digest,
        'source_head': source_head, 'channel': args.channel,
        'scope': 'isolated Omarchy; guest-local synthetic HTTPS release contract, NOT GitHub publication'}, indent=2) + '\n')
    command = [sys.executable, str(source.parent / 'gui/gui-vm.py'), '--template', str(args.template.resolve()),
               '--ovmf-vars', str(args.ovmf_vars.resolve()), '--ssh-key', str(args.ssh_key.resolve()),
               '--ssh-user', args.ssh_user, '--binary', str(args.appimage.resolve()),
               '--scenario', str(scenario_path), '--results', str(args.results / 'vm'),
               '--isolated-network', '--scenario-timeout', '360', '--job-timeout', '660']
    evidence = Evidence(args.results)
    error = None
    with (args.results / 'runner.log').open('wb') as log:
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT)
        position = 0
        deadline = time.monotonic() + 600
        try:
            while process.poll() is None:
                scenario_log = args.results / 'vm/scenario.log'
                if scenario_log.exists():
                    with scenario_log.open() as lines:
                        lines.seek(position)
                        while True:
                            line = lines.readline()
                            if not line or not line.endswith('\n'):
                                break
                            position = lines.tell()
                            if line.startswith('UPDATER_UI '):
                                request = json.loads(line.removeprefix('UPDATER_UI '))
                                print(request['label'], flush=True)
                                evidence.perform(request)
                                assert process.stdin is not None
                                process.stdin.write(b'OK\n')
                                process.stdin.flush()
                if time.monotonic() > deadline:
                    raise TimeoutError('GUI job exceeded 600 seconds')
                if evidence.samples and evidence.socket.exists():
                    evidence.frame()
                time.sleep(0.2)
        except Exception as failure:
            error = str(failure)
            print(error, file=sys.stderr)
            # Let the scenario fail normally and copy its evidence before shutdown.
            if process.stdin is not None:
                try:
                    process.stdin.write(b'FAIL\n')
                    process.stdin.flush()
                except BrokenPipeError:
                    pass
        finally:
            try:
                process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                process.terminate()
                process.wait(timeout=20)
            evidence.movie()
    scenario_log = args.results / 'vm/scenario.log'
    if not error and process.returncode == 0 and not candidate_matches(args.results / 'vm/artifacts/candidate.sha256', digest):
        error = 'guest candidate digest does not match the recorded AppImage'
    passed = not error and process.returncode == 0 and scenario_log.exists() and 'UPDATER_GUI_PASS' in scenario_log.read_text()
    (args.results / 'result.json').write_text(json.dumps({'passed': passed, 'error': error,
                                                          'runner_exit': process.returncode}, indent=2) + '\n')
    return 0 if passed else 1


if __name__ == '__main__':
    sys.exit(main())
