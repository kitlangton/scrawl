#!/usr/bin/env python3
"""Harmless local driver tests. NOT native GUI/audio/portal/candidate proof."""
import importlib.util
import math
import os
from pathlib import Path
import signal
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import wave

spec = importlib.util.spec_from_file_location('durability', Path(__file__).with_name('narration-durability.py'))
assert spec and spec.loader
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def wav(path, samples, rate=16000):
    with wave.open(str(path), 'wb') as output:
        output.setnchannels(1)
        output.setsampwidth(2)
        output.setframerate(rate)
        output.writeframes(struct.pack('<' + 'h' * len(samples), *samples))


class DriverTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir=os.environ.get('TMPDIR'))
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.out = self.root / 'artifacts'
        self.out.mkdir()
        self.driver = module.Scenario(self.root, self.out, Path(sys.executable).resolve(), self.root / 'tone.wav')

    def test_pcm_measures_frames_and_duration(self):
        samples = [int(8000 * math.sin(2 * math.pi * 440 * i / 16000)) for i in range(32000)]
        wav(self.root / 'tone.wav', samples)
        measured = module.require_audible(self.root / 'tone.wav')
        self.assertEqual(measured['frames'], 32000)
        self.assertEqual(measured['duration_seconds'], 2)
        self.assertGreater(measured['rms'], 5000)

    def test_silence_and_truncation_are_not_audio_proof(self):
        target = self.root / 'tone.wav'
        wav(target, [0] * 16000)
        with self.assertRaises(module.Failure):
            module.require_audible(target)
        target.write_bytes(target.read_bytes()[:-2])
        with self.assertRaises(module.Failure):
            module.pcm(target)

    def test_receipt_cannot_accept_other_capture_or_relative_path(self):
        good = {'capture_id': 'owned', 'status': 'saved', 'bundle_path': '/exact.apparent', 'error': None}
        self.assertEqual(module.receipt(good, 'owned', 'saved'), Path('/exact.apparent'))
        for changed in ({'capture_id': 'foreign'}, {'bundle_path': 'relative'}, {'error': 'failed'}, {'status': 'recording'}):
            with self.assertRaises(module.Failure):
                module.receipt(good | changed, 'owned', 'saved')
        with self.assertRaises(module.Failure):
            module.receipt(good | {'status': 'cancelled'}, 'owned', 'cancelled')

    def test_shell_entry_is_self_contained(self):
        result = subprocess.run(['bash', str(Path(__file__).with_name('narration-durability.py')), '--help'], capture_output=True, timeout=3)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(b'SCRAWL_GUI_ARTIFACTS', result.stdout)

    def test_exact_owned_child_signal_and_no_signal_after_reap(self):
        ready = self.root / 'child-ready'
        command = [str(self.driver.binary), '-c', f'import pathlib,time; pathlib.Path({str(ready)!r}).touch(); time.sleep(30)']
        child = self.driver.launch(command, 'harmless-child')
        try:
            self.driver.wait(ready.exists, 3, 'harmless child readiness')
            self.driver.signal_exact(child, signal.SIGTERM, command)
            self.assertEqual(self.driver.reap_owned(child, 3), -signal.SIGTERM)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=3)
        with self.assertRaises(module.Failure):
            self.driver.signal_exact(child, signal.SIGKILL, command)

    def test_wrong_argv_cannot_signal_even_owned_child(self):
        ready = self.root / 'child-ready'
        command = [str(self.driver.binary), '-c', f'import pathlib,time; pathlib.Path({str(ready)!r}).touch(); time.sleep(30)']
        child = self.driver.launch(command, 'harmless-child')
        try:
            self.driver.wait(ready.exists, 3, 'harmless child readiness')
            with self.assertRaises(module.Failure):
                self.driver.signal_exact(child, signal.SIGKILL, [self.driver.binary, 'app'])
            self.assertIsNone(child.poll())
        finally:
            child.terminate()
            child.wait(timeout=3)

    def test_independent_source_failure_prevents_all_app_proof(self):
        with patch.object(self.driver, 'prepare'), patch.object(self.driver, 'stimulus_start'), \
             patch.object(self.driver, 'stimulus_stop', side_effect=module.Failure('silent source')), \
             patch.object(self.driver, 'launch') as launch, patch.object(module.time, 'sleep'):
            with self.assertRaisesRegex(module.Failure, 'silent source'):
                self.driver.run()
        launch.assert_not_called()
        for name in ('normal', 'cancel', 'crash', 'sigterm', 'exact_crash_tail'):
            self.assertEqual(self.driver.report['gates'][name], 'UNRUN')

    def test_expired_deadline_never_launches_command(self):
        self.driver.end = 0
        with patch.object(module.subprocess, 'run') as run:
            with self.assertRaises(module.Failure):
                self.driver.call(['must-not-execute'])
        run.assert_not_called()

    def test_real_entrypoint_alarm_preserves_execution_and_cleanup_budgets(self):
        output = self.root / 'entrypoint'
        output.mkdir()
        environment = {'SCRAWL_GUI_OUTPUT': str(output),
                       'SCRAWL_GUI_BINARY': str(self.driver.binary),
                       'SCRAWL_GUI_AUDIO_FILE': str(self.root / 'tone.wav'),
                       'SCRAWL_GUI_ARTIFACTS': str(output / 'proof')}
        def completed():
            self.driver.report['result'] = 'PASS_RECIPE_GATES_ONLY'
        with patch.dict(module.os.environ, environment), \
             patch.object(module.sys, 'argv', ['narration-durability.py']), \
             patch.object(module, 'Scenario', return_value=self.driver), \
             patch.object(self.driver, 'run', side_effect=completed), \
             patch.object(self.driver, 'cleanup'), patch.object(self.driver, 'retain'), \
             patch.object(module.signal, 'signal'), patch.object(module.signal, 'alarm') as alarm:
            self.assertEqual(module.main(), 0)
        alarms = [call.args[0] for call in alarm.call_args_list]
        self.assertEqual(alarms, [module.EXECUTION_SECONDS + 5, module.CLEANUP_SECONDS, 0])
        self.assertGreaterEqual(alarms[0], 420)
        self.assertLessEqual(sum(alarms), 480)

    def test_native_duration_gate_rejects_short_but_nonzero_audio(self):
        wav(self.root / 'audio.wav', [1000] * 1600)
        self.driver.report['captures']['crash'] = {}
        with self.assertRaises(module.Failure):
            self.driver.native_audio('crash', self.root)
        self.assertEqual(self.driver.report['gates']['exact_crash_tail'], 'UNRUN')


if __name__ == '__main__':
    unittest.main()
