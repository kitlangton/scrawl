#!/usr/bin/env python3
"""Fast harness-contract tests; these do NOT establish GUI behavior."""
import importlib.util
from pathlib import Path
import hashlib
import unittest
import json
import re
import socket
import subprocess
import sys
import tempfile
import time
from unittest.mock import Mock, patch


def load(name):
    spec = importlib.util.spec_from_file_location(name.replace('-', '_'), Path(__file__).with_name(name + '.py'))
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class LocatorTests(unittest.TestCase):
    def setUp(self):
        self.gui = load('updater-gui')

    def test_phrase_uses_visible_word_bounds(self):
        words = [{'text': t, 'left': x, 'top': 10, 'width': w, 'height': 12,
                  'block_num': 1, 'par_num': 1, 'line_num': 1}
                 for t, x, w in [('Check', 10, 20), ('for', 32, 10), ('Updates', 44, 30)]]
        self.assertEqual(self.gui.find_phrase(words, 'Check for Updates'), (42, 16))
        with self.assertRaises(ValueError):
            self.gui.find_phrase(words, 'Updates')

    def test_ocr_preprocessing_removes_outline_but_preserves_short_text_strokes(self):
        from PIL import Image, ImageDraw
        image = Image.new('L', (90, 70), 20)
        draw = ImageDraw.Draw(image)
        draw.rectangle((5, 5, 75, 55), outline=230)
        draw.line((20, 30, 25, 30), fill=230)
        cleaned = self.gui.erase_long_light_lines(image)
        self.assertEqual(cleaned.getpixel((40, 5)), 20)
        self.assertEqual(cleaned.getpixel((5, 30)), 20)
        self.assertEqual(cleaned.getpixel((23, 30)), 230)
        self.assertEqual(image.getpixel((40, 5)), 230)

    def test_missing_or_ambiguous_phrase_fails_closed(self):
        with self.assertRaises(ValueError):
            self.gui.find_phrase([], 'Update now')
        word = {'text': 'OK', 'left': 10, 'top': 10, 'width': 20, 'height': 12,
                'block_num': 1, 'par_num': 1, 'line_num': 1}
        with self.assertRaises(ValueError):
            self.gui.find_phrase([word, dict(word, line_num=2)], 'OK')

    def test_distinct_controls_on_same_ocr_line_are_not_one_label(self):
        words = [{'text': t, 'left': x, 'top': 10, 'width': w, 'height': 56 if x >= 168 else 19,
                  'block_num': 1, 'par_num': 1, 'line_num': 1}
                 for t, x, w in [('Update', 10, 60), ('later', 78, 42),
                                 ('Update', 168, 60), ('now', 236, 35)]]
        self.assertEqual(self.gui.find_phrase(words, 'Update later'), (65, 19))
        self.assertEqual(self.gui.find_phrase(words, 'Update now'), (219, 38))
        with self.assertRaises(ValueError):
            self.gui.find_phrase(words, 'later')

    def test_ignored_later_and_escape_cannot_satisfy_dismissal(self):
        words = [{'text': text, 'left': 10, 'top': row * 30, 'width': 160, 'height': 19,
                  'block_num': 1, 'par_num': 1, 'line_num': row}
                 for row, text in enumerate(['Check for Updates', 'Update available', 'Update now'])]
        # The former positive-only proof succeeds even with the modal still open.
        self.assertEqual(self.gui.find_phrase(words, 'Check for Updates'), (90, 9))
        for action in ['Later', 'Escape']:
            with self.subTest(action=action), tempfile.TemporaryDirectory() as directory:
                evidence = self.gui.Evidence(Path(directory))
                with patch.object(evidence, 'frame', return_value=Path(directory) / 'frame.png'), \
                     patch.object(evidence, 'words', return_value=words):
                    with self.assertRaisesRegex(RuntimeError, 'dialog did not disappear'):
                        evidence.perform({'op': 'assert-dismissed', 'timeout': 0, 'label': action})

    def test_dismissal_requires_visible_settings_and_no_modal_in_either_ocr_pass(self):
        background = [{'text': 'Check for Updates', 'left': 10, 'top': 10, 'width': 160,
                       'height': 19, 'block_num': 1, 'par_num': 1, 'line_num': 1}]
        modal = dict(background[0], text='Update available', line_num=2)
        self.assertTrue(self.gui.dialog_dismissed([background, background]))
        self.assertFalse(self.gui.dialog_dismissed([[], []]))
        self.assertFalse(self.gui.dialog_dismissed([background, background + [modal]]))


class CandidateTests(unittest.TestCase):
    def test_capture_restart_click_requires_ack_and_bounds_focus_retries(self):
        scenario = Path(__file__).with_name('updater-gui-scenario.sh').read_text()
        function = scenario.split('  click_update_guard() {\n', 1)[1].split('\n  }\n', 1)[0]
        for acknowledged_at, expected_clicks, success in [(1, 1, True), (2, 2, True), (99, 3, False)]:
            with self.subTest(acknowledged_at=acknowledged_at), tempfile.TemporaryDirectory() as temporary:
                script = '''out=$1; acknowledged_at=$2; clicks=0
ui() {
    clicks=$((clicks+1))
    if [ "$clicks" -eq "$acknowledged_at" ]; then
        echo 'Update restart deferred until capture is idle' >> "$out/app.log"
    fi
}
sleep() { :; }
click_update_guard() {
''' + function + '''
}
click_update_guard; result=$?
echo "$clicks"
exit "$result"
'''
                result = subprocess.run(['bash', '-c', script, 'focus-retry-test', temporary, str(acknowledged_at)],
                                        capture_output=True, text=True, timeout=5)
                self.assertEqual(result.returncode == 0, success, result.stderr)
                self.assertEqual(result.stdout.strip(), str(expected_clicks))

    def test_capture_guard_requires_owned_admitted_receipt_and_active_state(self):
        scenario = Path(__file__).with_name('updater-gui-scenario.sh').read_text()
        source = scenario.split('cat > "$out/assert-owned-capture.py" <<\'PY\'\n', 1)[1].split('\nPY', 1)[0]
        validator = compile(source, 'scenario-owned-capture-validator', 'exec')
        owned = 'c' * 32
        cases = [
            ('starting', owned, None, 'starting', owned, True),
            ('starting', owned, None, 'selecting', owned, True),
            ('recording', owned, None, 'recording', owned, True),
            ('starting', 'd' * 32, None, 'starting', owned, False),
            ('starting', owned, None, 'starting', 'd' * 32, False),
            ('starting', owned, 'start_failed', 'starting', owned, False),
            ('failed', owned, None, 'starting', owned, False),
            ('unknown', owned, None, 'starting', owned, False),
            ('rejected', owned, None, 'idle', None, False),
            ('cancelled', owned, None, 'idle', None, False),
            ('recording', owned, None, 'processing', owned, False),
            ('recording', owned, None, 'idle', None, False),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            receipt = Path(temporary) / 'receipt.json'
            state = Path(temporary) / 'state.json'
            for status, receipt_id, error, phase, state_id, accepted in cases:
                with self.subTest(status=status, receipt_id=receipt_id, phase=phase, state_id=state_id, error=error):
                    receipt.write_text(json.dumps({'capture_id': receipt_id, 'status': status, 'error': error}))
                    state.write_text(json.dumps({'state': {'capture_id': state_id, 'phase': phase}}))
                    with patch.object(sys, 'argv', ['validator', owned, str(receipt), str(state)]):
                        if accepted:
                            exec(validator, {})
                        else:
                            with self.assertRaises(AssertionError):
                                exec(validator, {})

    def test_controller_generates_both_channels_and_uses_relocated_vm_runner(self):
        gui = load('updater-gui')
        for channel in ['stable', 'main']:
            with self.subTest(channel=channel), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                image = directory / 'candidate.AppImage'
                image.write_bytes(b'\x7fELF' + b'\0' * 4 + b'AI\x02')
                results = directory / 'results'
                argv = ['updater-gui.py', '--template', str(directory / 'template'),
                        '--ovmf-vars', str(directory / 'vars'), '--ssh-key', str(directory / 'key'),
                        '--ssh-user', 'fixture', '--appimage', str(image), '--results', str(results)]
                if channel == 'main':
                    argv.extend(['--channel', channel])
                # Stop at the runner boundary: this test must never launch a VM.
                process = Mock(returncode=1)
                process.poll.return_value = 1

                def git_output(command, **kwargs):
                    self.assertEqual(command[:2], ['git', '-C'])
                    if command[3:] == ['rev-parse', 'HEAD']:
                        return 'a' * 40
                    self.assertEqual(command[3:], ['status', '--porcelain', '--untracked-files=all'])
                    self.assertEqual(kwargs, {'text': True})
                    return ''

                with patch.object(sys, 'argv', argv), \
                     patch.object(gui.subprocess, 'check_output', side_effect=git_output), \
                     patch.object(gui.subprocess, 'Popen', return_value=process) as launch:
                    self.assertEqual(gui.main(), 1)
                command = launch.call_args.args[0]
                expected = Path(__file__).resolve().parent.parent / 'gui/gui-vm.py'
                self.assertEqual(Path(command[1]), expected)
                self.assertTrue(expected.is_file())
                self.assertIn('--isolated-network', command)
                self.assertEqual(json.loads((results / 'candidate.json').read_text())['channel'], channel)
                scenario = (results / 'scenario.sh').read_text()
                self.assertIn("export UPDATER_CHANNEL='" + channel + "'", scenario)
                self.assertNotIn('__UPDATER_', scenario)
                subprocess.run(['bash', '-n', str(results / 'scenario.sh')], check=True)
                for index, source in enumerate(re.findall(r"<<'PY'\n(.*?)\nPY", scenario, re.S)):
                    compile(source, f'generated-{channel}-{index}', 'exec')
                fixture = scenario.split("<<'UPDATER_FIXTURE_EOF'\n", 1)[1].split('\nUPDATER_FIXTURE_EOF', 1)[0]
                compile(fixture, f'generated-{channel}-fixture', 'exec')

    def test_transferred_digest_must_equal_the_recorded_candidate(self):
        gui = load('updater-gui')
        expected = hashlib.sha256(b'exact packaged candidate').hexdigest()
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'candidate.sha256'
            self.assertFalse(gui.candidate_matches(path, expected))
            path.write_text(expected + '\n')
            self.assertTrue(gui.candidate_matches(path, expected))
            path.write_text(hashlib.sha256(b'different package with same revision').hexdigest() + '\n')
            self.assertFalse(gui.candidate_matches(path, expected))


    def test_dirty_repository_refuses_before_vm_launch_including_docs_and_tooling(self):
        gui = load('updater-gui')
        real_check_output = subprocess.check_output
        real_popen = subprocess.Popen
        for path in ['capture/desktop/linux/updates/update.rs', 'capture/desktop/mod.rs',
                     'capture/desktop/settings/settings.rs', 'shared/handoff.rs',
                     'composition/command_line.rs', 'capture/desktop/docs/linux/linux.md',
                     'capture/desktop/scripts/linux/updater/updater-gui.py']:
            with self.subTest(path=path), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                repository = directory / 'repository'
                repository.mkdir()
                subprocess.run(['git', 'init', '-q', str(repository)], check=True)
                dirty = repository / path
                dirty.parent.mkdir(parents=True, exist_ok=True)
                dirty.write_text('uncommitted candidate change\n')
                if path.endswith('.rs'):
                    subprocess.run(['git', '-C', str(repository), 'add', path], check=True)
                image = directory / 'candidate.AppImage'
                image.write_bytes(b'\x7fELF' + b'\0' * 4 + b'AI\x02')
                results = directory / 'results'
                argv = ['updater-gui.py', '--template', str(directory / 'template'),
                        '--ovmf-vars', str(directory / 'vars'), '--ssh-key', str(directory / 'key'),
                        '--ssh-user', 'fixture', '--appimage', str(image), '--results', str(results)]

                def git_output(command, **kwargs):
                    self.assertEqual(command[:2], ['git', '-C'])
                    if command[3:] == ['rev-parse', 'HEAD']:
                        return 'a' * 40
                    self.assertEqual(command[3], 'status')
                    with patch.object(gui.subprocess, 'Popen', real_popen):
                        return real_check_output(command[:2] + [str(repository)] + command[3:], **kwargs)

                with patch.object(sys, 'argv', argv), \
                     patch.object(gui.subprocess, 'check_output', side_effect=git_output), \
                     patch.object(gui.subprocess, 'Popen', return_value=Mock(returncode=1)) as launch, \
                     patch.object(sys, 'stderr') as stderr:
                    with self.assertRaises(SystemExit) as error:
                        gui.main()
                    self.assertEqual(error.exception.code, 2)
                    self.assertIn('repository must be clean', ''.join(c.args[0] for c in stderr.write.call_args_list))
                    launch.assert_not_called()
                self.assertFalse((results / 'scenario.sh').exists())
                self.assertFalse((results / 'candidate.json').exists())


class FixtureTests(unittest.TestCase):
    def setUp(self):
        self.fixture = load('updater-fixture')

    def test_main_pagination_canonical_assets_and_current_identity(self):
        source = 'a' * 40
        control = {'channel': 'main', 'source_head': source, 'version': '1.2.3', 'mode': 'current'}
        base = '/repos/tryapparent/apparent/releases?per_page=10&page='
        first = json.loads(self.fixture.response(control, base + '1', b'x')[1])
        self.assertFalse(first[0]['prerelease'])
        release = json.loads(self.fixture.response(control, base + '2', b'x')[1])[0]
        self.assertEqual(release['tag_name'], 'linux-main-' + source)
        self.assertTrue(release['prerelease'])
        self.assertFalse(release['draft'])
        self.assertTrue(release['published_at'])
        self.assertEqual(json.loads(self.fixture.response(control, base + '3', b'x')[1]), [])
        for mode in ['available', 'bad-checksum', 'diverged']:
            control['mode'] = mode
            release = json.loads(self.fixture.response(control, base + '2', b'x')[1])[0]
            self.assertEqual(release['tag_name'], 'linux-main-' + 'b' * 40)
            for asset in release['assets']:
                self.assertEqual(asset['browser_download_url'],
                                 'https://github.com/tryapparent/apparent/releases/download/'
                                 + release['tag_name'] + '/' + asset['name'])
            status, body = self.fixture.response(control, release['assets'][1]['browser_download_url'].split('github.com')[1], b'x')
            self.assertEqual(status, 200)
            self.assertEqual(body.split()[0], b'0' * 64 if mode == 'bad-checksum' else hashlib.sha256(b'x').hexdigest().encode())

    def test_main_compare_direction_divergence_and_fail_closed_routes(self):
        control = {'channel': 'main', 'source_head': 'a' * 40, 'version': '1.2.3', 'mode': 'available'}
        base = '/repos/tryapparent/apparent/compare/'
        query = '?per_page=1&page=2'
        forward = base + 'a' * 40 + '...' + 'b' * 40 + query
        reverse = base + 'b' * 40 + '...' + 'a' * 40 + query
        self.assertEqual(json.loads(self.fixture.response(control, forward, b'')[1])['status'], 'ahead')
        self.assertEqual(json.loads(self.fixture.response(control, forward, b'')[1])['base_commit']['sha'], 'a' * 40)
        self.assertEqual(json.loads(self.fixture.response(control, reverse, b'')[1])['status'], 'behind')
        self.assertEqual(json.loads(self.fixture.response(control, reverse, b'')[1])['base_commit']['sha'], 'b' * 40)
        control['mode'] = 'diverged'
        self.assertEqual(json.loads(self.fixture.response(control, forward, b'')[1])['status'], 'diverged')
        self.assertEqual(self.fixture.response(control, base + 'c' * 40 + '...' + 'b' * 40, b'')[0], 404)
        control['mode'] = 'error'
        self.assertEqual(self.fixture.response(control, '/repos/tryapparent/apparent/releases?per_page=10&page=1', b'')[0], 503)
        self.assertEqual(self.fixture.response(control, '/repos/tryapparent/apparent/releases?per_page=10&page=0', b'')[0], 404)

    def test_release_contract_and_digest_are_exact(self):
        import hashlib
        payload = b'test AppImage bytes, not a published release'
        control = {'version': '1.2.3', 'mode': 'available'}
        status, body = self.fixture.response(control, '/repos/tryapparent/apparent/releases/latest', payload)
        import json
        release = json.loads(body)
        self.assertEqual(status, 200)
        self.assertEqual(release['tag_name'], 'v999.0.0')
        url = release['assets'][1]['browser_download_url']
        status, checksum = self.fixture.response(control, url.split('github.com')[1], payload)
        self.assertEqual(status, 200)
        self.assertEqual(checksum.decode(), hashlib.sha256(payload).hexdigest() + '  Apparent-999.0.0-linux-x86_64.AppImage\n')

    def test_only_expected_routes_and_modes(self):
        self.assertEqual(self.fixture.response({'version': '1.2.3', 'mode': 'error'},
                         '/repos/tryapparent/apparent/releases/latest', b'')[0], 503)
        self.assertEqual(self.fixture.response({'version': '1.2.3', 'mode': 'current'},
                         '/unexpected', b'')[0], 404)
        self.assertEqual(self.fixture.response({'version': '1.2.3', 'mode': 'bad-checksum'},
                         '/tryapparent/apparent/releases/download/v999.0.0/Apparent-999.0.0-linux-x86_64.AppImage.sha256', b'x')[1].split()[0], b'0' * 64)

    def test_real_curl_connect_requires_trusted_tls_and_refuses_foreign_hosts(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / 'payload').write_bytes(b'fixture bytes')
            (directory / 'control.json').write_text(json.dumps({'version':'1.2.3','mode':'current','serial':1}))
            subprocess.run(['openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
                            '-subj', '/CN=api.github.com', '-addext', 'subjectAltName=DNS:api.github.com,DNS:github.com',
                            '-keyout', str(directory / 'key.pem'), '-out', str(directory / 'cert.pem')],
                           check=True, capture_output=True, timeout=10)
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', 0))
                port = sock.getsockname()[1]
            server = subprocess.Popen([sys.executable, str(Path(__file__).with_name('updater-fixture.py')),
                                       str(directory), str(directory / 'payload'), '--port', str(port)],
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            try:
                deadline = time.monotonic() + 5
                while not (directory / 'server-ready').exists():
                    if server.poll() is not None or time.monotonic() > deadline:
                        self.fail('fixture server did not start')
                    time.sleep(.05)
                curl = ['curl', '--silent', '--show-error', '--fail', '--max-time', '3',
                        '--proxy', f'http://127.0.0.1:{port}', '--noproxy', '']
                url = 'https://api.github.com' + self.fixture.API
                untrusted = subprocess.run(curl + [url], capture_output=True)
                self.assertEqual(untrusted.returncode, 60)
                trusted = subprocess.run(curl + ['--cacert', str(directory / 'cert.pem'), url],
                                         check=True, capture_output=True)
                self.assertEqual(json.loads(trusted.stdout)['tag_name'], 'v1.2.3')
                foreign = subprocess.run(curl + ['--cacert', str(directory / 'cert.pem'), 'https://foreign.invalid/'],
                                         capture_output=True)
                self.assertNotEqual(foreign.returncode, 0)
                self.assertIn(b'403', foreign.stderr)
            finally:
                server.terminate()
                server.wait(timeout=5)


if __name__ == '__main__':
    unittest.main()
