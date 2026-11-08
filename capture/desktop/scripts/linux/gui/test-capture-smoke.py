#!/usr/bin/env python3
"""Harmless subprocess transport tests. NO native app, X11, portal or media proof."""
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("capture_smoke", Path(__file__).with_name("capture-smoke.py"))
assert spec and spec.loader
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)

# This is an explicitly synthetic CLI peer, not a fake native acceptance result.
PEER = r'''#!/usr/bin/env python3
import json, os, pathlib, socket, sys, time
root = pathlib.Path(os.environ['SCRAWL_GUI_OUTPUT'])
if sys.argv[1:] == ['app']:
    sockets = []
    for relative in ('.config/apparent/agent-control/capture.sock', '.config/apparent/launch/open.sock'):
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        sock = socket.socket(socket.AF_UNIX)
        sock.bind(str(path))
        sockets.append(sock)
    while True: time.sleep(1)
args = sys.argv[2:]
f = root / 'peer-state.json'
s = json.loads(f.read_text()) if f.exists() else {'state': {'phase': 'idle', 'capture_id': None}, 'receipts': {}}
op = args[0]
if op == 'state':
    reply = {'state': s['state']}
elif op == 'start':
    owned = args[args.index('--request-id') + 1]
    s['state'] = {'phase': 'recording', 'capture_id': owned}
    reply = s['receipts'][owned] = {'capture_id': owned, 'status': 'recording', 'bundle_path': None, 'error': None}
    if os.environ.get('PEER_PENDING'):
        s['receipts'][owned]['status'] = 'starting'
        reply = {'capture_id': owned, 'error': 'timeout'}
elif op == 'stop':
    owned = args[1]
    if os.environ.get('PEER_FAIL'):
        print('synthetic stop stderr', file=sys.stderr)
        print(json.dumps({'capture_id': owned, 'error': 'finalization_failed', 'message': 'synthetic stop failure'}))
        sys.exit(7)
    bundle = pathlib.Path(os.environ['APPARENT_CAPTURES_DIR']) / 'synthetic.apparent'
    (bundle / 'frames').mkdir(parents=True)
    for name, content in {'apparent.json': '{}', 'summary.md': 'SYNTHETIC', 'contact-sheet.png': 'NOT AN IMAGE', 'recording.mp4': 'NOT MEDIA', 'frames/frame-01.png': 'NOT AN IMAGE'}.items():
        (bundle / name).write_text(content)
    s['state'] = {'phase': 'completed', 'capture_id': None}
    reply = s['receipts'][owned] = {'capture_id': owned, 'status': 'saved', 'bundle_path': str(bundle), 'error': None}
elif op == 'cancel':
    owned = args[1]
    s['state'] = {'phase': 'idle', 'capture_id': None}
    reply = s['receipts'][owned] = {'capture_id': owned, 'status': 'cancelled', 'bundle_path': None, 'error': None}
else:
    reply = s['receipts'][args[1]]
f.write_text(json.dumps(s))
print(json.dumps(reply))
'''


class SyntheticRecipe(smoke.Recipe):
    def launch(self, argv, log):
        if '--scene' in argv:
            # Only the scene producer is replaced. CLI calls use actual subprocesses,
            # file persistence, stdout/stderr and exits through the real recipe.
            code = "import pathlib,sys,time; pathlib.Path(sys.argv[1]).write_text('{\"synthetic\":true}'); time.sleep(120)"
            argv = [sys.executable, '-c', code, argv[-1]]
        return super().launch(argv, log)


class ParserTests(unittest.TestCase):
    def test_state_requires_known_phase_and_identity_type(self):
        for value in ({}, {'state': []}, {'state': {'phase': 'ended'}}, {'state': {'phase': 'idle'}}, {'state': {'phase': 'recording', 'capture_id': 1}}):
            with self.assertRaises(smoke.Failure):
                smoke.state(value)
        self.assertEqual(smoke.state({'state': {'phase': 'completed', 'capture_id': None}})['phase'], 'completed')

    def test_receipt_requires_exact_owned_id_and_saved_path(self):
        good = {'capture_id': 'owned', 'status': 'saved', 'bundle_path': '/private/exact.apparent', 'error': None}
        self.assertEqual(smoke.receipt(good, 'owned', 'saved'), '/private/exact.apparent')
        for changes in ({'capture_id': 'other'}, {'status': 'processing'}, {'bundle_path': None}, {'bundle_path': 'relative'}, {'error': 'finalization_failed'}):
            with self.assertRaises(smoke.Failure):
                smoke.receipt(good | changes, 'owned', 'saved')
        with self.assertRaises(smoke.Failure):
            smoke.receipt({'state': {'phase': 'completed', 'capture_id': None}}, 'owned', 'saved')

    def test_cancel_cannot_claim_bundle(self):
        value = {'capture_id': 'owned', 'status': 'cancelled', 'bundle_path': None, 'error': None}
        self.assertIsNone(smoke.receipt(value, 'owned', 'cancelled'))
        with self.assertRaises(smoke.Failure):
            smoke.receipt(value | {'bundle_path': '/wrong'}, 'owned', 'cancelled')

    def test_runner_bash_entry(self):
        result = subprocess.run(['bash', str(Path(__file__).with_name('capture-smoke.py')), '--help'], capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('--consent-timeout', result.stdout)


class CleanupTests(unittest.TestCase):
    def test_reaped_peer_never_signalled_or_probed_and_live_peer_is_joined(self):
        with tempfile.TemporaryDirectory(prefix='cs-', dir=os.environ.get('TMPDIR')) as directory:
            recipe = smoke.Recipe(Path(directory), Path('/unused'), 1)
            stale = recipe.launch([sys.executable, '-c', 'pass'], 'stale.log')
            stale.wait(timeout=3)
            live = recipe.launch([sys.executable, '-c', 'import time; time.sleep(120)'], 'live.log')
            real_killpg = os.killpg
            def owned_only(pid, sig):
                # Never deliver even a probe to the stale numeric ID on old code.
                if pid != stale.pid:
                    return real_killpg(pid, sig)
            try:
                with patch.object(smoke.os, 'killpg', side_effect=owned_only) as signals:
                    recipe.cleanup()
                self.assertFalse(any(call.args[0] == stale.pid for call in signals.call_args_list))
                self.assertIsNotNone(live.returncode)
                self.assertEqual(recipe.report['gates']['owned_process_cleanup'], 'FAIL')
                self.assertTrue(any('ownership' in error for error in recipe.report['cleanup_errors']))
            finally:
                if live.poll() is None:
                    live.kill()
                live.wait(timeout=3)

    def check_anchored_cleanup(self, ignore_term=False, exited=False):
        with tempfile.TemporaryDirectory(prefix='cs-', dir=os.environ.get('TMPDIR')) as directory:
            root = Path(directory)
            recipe = smoke.Recipe(root, Path('/unused'), 1)
            ready = root / 'ready'
            code = ('import signal,pathlib,sys,time; '
                    f'signal.signal(signal.SIGTERM, signal.{"SIG_IGN" if ignore_term else "SIG_DFL"}); '
                    'pathlib.Path(sys.argv[1]).touch(); ' + ('pass' if exited else 'time.sleep(120)'))
            job = recipe.launch([sys.executable, '-c', code, str(ready)], 'peer.log')
            real_killpg = os.killpg
            def anchored(pid, sig):
                self.assertIsNone(job.returncode, 'group operation after Popen reap')
                # WNOWAIT confirms that this exact child is still ours, even exited.
                os.waitid(os.P_PID, pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
                return real_killpg(pid, sig)
            try:
                recipe.wait(ready.exists, 3, 'peer not ready')
                if exited:
                    recipe.wait(lambda: os.waitid(os.P_PID, job.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT) is not None,
                                3, 'peer did not exit')
                with patch.object(smoke.os, 'killpg', side_effect=anchored) as signals:
                    recipe.cleanup()
                self.assertIsNotNone(job.returncode)
                self.assertEqual(recipe.report['gates']['owned_process_cleanup'], 'PASS', recipe.report)
                self.assertIn(signal.SIGTERM, [call.args[1] for call in signals.call_args_list])
                # A /proc snapshot cannot authorize skipping the final group
                # kill: descendants can fork while that snapshot is enumerated.
                self.assertIn(signal.SIGKILL, [call.args[1] for call in signals.call_args_list])
            finally:
                if job.poll() is None:
                    job.kill()
                job.wait(timeout=3)

    def test_live_peer_stopped_and_joined(self):
        self.check_anchored_cleanup()

    def test_missing_kernel_child_anchor_refuses_all_group_operations(self):
        with tempfile.TemporaryDirectory(prefix='cs-', dir=os.environ.get('TMPDIR')) as directory:
            recipe = smoke.Recipe(Path(directory), Path('/unused'), 1)
            job = recipe.launch([sys.executable, '-c', 'pass'], 'peer.log')
            # Reap outside Popen, leaving returncode unset: checking the cached
            # attribute alone cannot establish ownership of the numeric PID.
            os.waitpid(job.pid, 0)
            with patch.object(smoke.os, 'killpg') as signals:
                recipe.cleanup()
            signals.assert_not_called()
            self.assertEqual(recipe.report['gates']['owned_process_cleanup'], 'FAIL')
            job.wait(timeout=3)

    def test_timeout_kill_retains_unreaped_anchor(self):
        self.check_anchored_cleanup(ignore_term=True)

    def test_exited_but_unreaped_leader_retains_anchor(self):
        self.check_anchored_cleanup(exited=True)


class TransportTests(unittest.TestCase):
    def exercise(self, fail=False, pending=False):
        # Keep paths short, as required by the real Unix control socket contract.
        with tempfile.TemporaryDirectory(prefix='cs-', dir=os.environ.get('TMPDIR')) as directory:
            root = Path(directory)
            out = root / 'o'
            out.mkdir()
            tools = root / 'bin'
            tools.mkdir()
            runtime = root / 'runtime'
            runtime.mkdir()
            binary = tools / 'peer'
            binary.write_text(PEER)
            binary.chmod(0o700)
            systemctl = tools / 'systemctl'
            systemctl.write_text('#!/bin/sh\nprintf "DISPLAY=:synthetic\\nXDG_RUNTIME_DIR=' + str(runtime) + '\\n"\n')
            systemctl.chmod(0o700)
            env = {'PATH': str(tools) + os.pathsep + os.environ['PATH'], 'SCRAWL_GUI_OUTPUT': str(out)}
            if fail:
                env['PEER_FAIL'] = '1'
            if pending:
                env['PEER_PENDING'] = '1'
            with patch.dict(os.environ, env):
                recipe = SyntheticRecipe(out, binary, 1)
                try:
                    if pending:
                        with self.assertRaisesRegex(smoke.Failure, 'consent versus backend failure is unresolved'):
                            recipe.run()
                    elif fail:
                        with self.assertRaisesRegex(smoke.Failure, 'synthetic stop failure'):
                            recipe.run()
                    else:
                        recipe.run()
                finally:
                    recipe.cleanup()
            report = json.loads((out / 'capture-smoke-report.json').read_text())
            self.assertEqual(report['gates']['owned_process_cleanup'], 'PASS', report)
            self.assertFalse((out / '.config/apparent/agent-control/capture.sock').exists())
            self.assertFalse((out / '.config/apparent/launch/open.sock').exists())
            self.assertEqual(report['gates']['selected_target_pixels'], 'UNRUN')
            self.assertEqual(report['gates']['movie_nonblack_changing'], 'UNRUN')
            if pending:
                self.assertEqual(len(report['requests']), 1)
                peer = json.loads((out / 'peer-state.json').read_text())
                self.assertEqual(peer['receipts'][recipe.ids[0]]['status'], 'cancelled')
                self.assertEqual(report['gates']['saved_receipt'], 'UNRUN')
                self.assertEqual(report['pending_receipt']['status'], 'starting')
            elif fail:
                failed = [entry for entry in report['commands'] if entry.get('exit') == 7]
                self.assertEqual(len(failed), 1)
                self.assertIn('synthetic stop stderr', (out / failed[0]['stderr']).read_text())
                self.assertIn('synthetic stop failure', (out / failed[0]['stdout']).read_text())
                self.assertEqual(report['gates']['cancelled_receipt'], 'UNRUN')
                peer = json.loads((out / 'peer-state.json').read_text())
                self.assertEqual(peer['receipts'][recipe.ids[0]]['status'], 'cancelled')
            else:
                self.assertEqual(report['gates']['saved_receipt'], 'PASS')
                self.assertEqual(set(report['removed_private_sockets']), {'.config/apparent/agent-control/capture.sock', '.config/apparent/launch/open.sock'})
                self.assertEqual(report['gates']['cancelled_receipt'], 'PASS')
                self.assertEqual(report['saved_bundle_relative_to_output'], 'captures/synthetic.apparent')
                self.assertEqual(len(report['requests']), 2)
                self.assertNotEqual(*[r['capture_id'] for r in report['requests']])
                self.assertEqual(report['gates']['movie_full_decode'], 'UNRUN')

    def test_whole_recipe_with_real_harmless_transport(self):
        self.exercise()

    def test_failed_stop_streams_survive_owned_cancel_cleanup(self):
        self.exercise(fail=True)

    def test_pending_timeout_cancels_same_id_without_new_take(self):
        self.exercise(pending=True)


if __name__ == '__main__':
    unittest.main()
