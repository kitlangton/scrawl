"""Selector contracts and real Git deltas; no compiler/native work in fixtures."""
import importlib.util
import os
from pathlib import Path
import subprocess
import select
import signal
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = Path(__file__).resolve().parents[1] / "verification.py"
spec = importlib.util.spec_from_file_location("verification", SCRIPT)
assert spec is not None and spec.loader is not None
verification = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verification)


class SelectionTests(unittest.TestCase):
    def setUp(self):
        guard = patch.object(verification, "check_temp_environment")
        guard.start()
        self.addCleanup(guard.stop)

    def test_documents_do_not_compile_app(self):
        result = verification.select(["enablement/refactor.md", "AGENTS.md", "docs/capture.md"])
        self.assertFalse(result["root"])
        self.assertFalse(result["engine"])

    def test_explicit_platform_suite_executes_only_its_commands_and_default_keeps_all(self):
        for suite, cargo, browser in [('all', True, True), ('rust', True, False), ('browser', False, True)]:
            args = ['verification.py', '--base', 'unknown', '--run']
            if suite != 'all': args += ['--suite', suite]
            with self.subTest(suite=suite), patch.object(verification.sys, 'argv', args), patch.object(
                    verification, 'plan', return_value=verification.select(['build.rs', 'connect/web/package.json'])), patch.object(
                    verification.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, stdout='test result: ok. 1 passed; 0 failed\n')) as run:
                verification.main()
                commands = [call.args[0] for call in run.call_args_list]
                self.assertEqual(any('cargo' in command for command in commands), cargo)
                self.assertEqual(any(command[0] == 'bun' for command in commands), browser)
                self.assertEqual(['node', 'scripts/test-browser.mjs'] in commands, browser)
                self.assertEqual(any('capture/engine/Cargo.toml' in command for command in commands), cargo)

    def test_receipt_wrapper_preserves_explicit_suite(self):
        args = ['verification.py', '--base', 'unknown', '--run', '--online',
                '--suite', 'rust', '--parallel-contracts', '--overlap-root-build',
                '--cargo-timings', '--receipt-dir', '/external/receipts']
        with patch.object(verification.sys, 'argv', args), patch.object(verification.subprocess, 'run') as run:
            verification.main()
            command = run.call_args.args[0]
            self.assertEqual(command[command.index('--suite', command.index('--') + 1) + 1], 'rust')
            self.assertEqual(command[command.index('--suite') + 1], 'platform-rust')
            self.assertIn('--online', command)
            self.assertIn('--parallel-contracts', command[command.index('--') + 1:])
            self.assertIn('--overlap-root-build', command[command.index('--') + 1:])
            self.assertIn('--cargo-timings', command[command.index('--') + 1:])

    def test_overlap_root_build_is_explicit_and_forwarded(self):
        for enabled in (False, True):
            args = ['verification.py', '--base', 'unknown', '--run']
            if enabled:
                args.append('--overlap-root-build')
            with self.subTest(enabled=enabled), patch.object(verification.sys, 'argv', args), patch.object(
                    verification, 'plan', return_value={'root': True, 'engine': False, 'browser': False, 'focused': []}), patch.object(
                    verification, 'run_checks') as run:
                verification.main()
                self.assertEqual(run.call_args.kwargs['overlap_root_build'], enabled)

    def test_parallel_contracts_overlap_selected_checks_and_join_both_results(self):
        real_spawn = subprocess.Popen
        for outcome in ('success', 'contracts-fail', 'selected-fail', 'signal', 'spawn-signal'):
            with self.subTest(outcome=outcome):
                ready_read, ready_write = os.pipe()
                release_read, release_write = os.pipe()
                processes = []
                previous = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT)}
                def spawn(command, **kwargs):
                    self.assertEqual(command[-1], '--contract-checks')
                    self.assertTrue(kwargs['start_new_session'])
                    program = 'import os,sys; os.write(int(sys.argv[1]), b"R"); os.read(int(sys.argv[2]), 1); sys.exit(int(sys.argv[3]))'
                    process = real_spawn([sys.executable, '-c', program, str(ready_write), str(release_read),
                                          '7' if outcome == 'contracts-fail' else '0'],
                                         pass_fds=(ready_write, release_read), **kwargs)
                    processes.append(process)
                    if outcome == 'spawn-signal':
                        os.kill(os.getpid(), signal.SIGTERM)
                    return process
                def selected(selection, offline, cargo_timings, overlap_root_build=False):
                    self.assertFalse(offline)
                    self.assertEqual(selection, {'root': True, 'engine': False})
                    self.assertTrue(select.select([ready_read], [], [], 5)[0], 'contracts did not start before selected checks')
                    self.assertEqual(os.read(ready_read, 1), b'R')
                    self.assertIsNone(processes[0].poll(), 'contract lane must still be running')
                    if outcome == 'selected-fail':
                        raise RuntimeError('selected checks failed')
                    if outcome == 'signal':
                        os.kill(os.getpid(), signal.SIGTERM)
                    os.write(release_write, b'G')
                try:
                    with patch.object(verification.subprocess, 'Popen', side_effect=spawn), patch.object(
                            verification.subprocess, 'run'), patch.object(verification, 'run_selected_checks', side_effect=selected):
                        def run():
                            verification.run_checks({'root': True, 'engine': False}, offline=False, parallel_contracts=True)
                        if outcome == 'success':
                            run()
                        elif outcome == 'contracts-fail':
                            with self.assertRaises(subprocess.CalledProcessError) as error:
                                run()
                            self.assertEqual(error.exception.returncode, 7)
                        elif outcome == 'selected-fail':
                            with self.assertRaisesRegex(RuntimeError, 'selected checks failed'):
                                run()
                        else:
                            with self.assertRaises(SystemExit) as error:
                                run()
                            self.assertEqual(error.exception.code, 128 + signal.SIGTERM)
                    self.assertIsNotNone(processes[0].poll(), 'owned contracts must be reaped')
                    self.assertEqual({sig: signal.getsignal(sig) for sig in previous}, previous)
                finally:
                    for process in processes:
                        if process.poll() is None:
                            os.killpg(process.pid, signal.SIGKILL)
                            process.wait()
                    for fd in (ready_read, ready_write, release_read, release_write):
                        os.close(fd)

    def test_parallel_contracts_restore_handlers_when_spawn_fails(self):
        previous = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT)}
        with patch.object(verification.subprocess, 'Popen', side_effect=OSError('spawn failed')):
            with self.assertRaisesRegex(OSError, 'spawn failed'), verification.overlapping_contracts():
                self.fail('selected checks must not run')
        self.assertEqual({sig: signal.getsignal(sig) for sig in previous}, previous)

    def test_cargo_timings_preserve_commands_and_test_arguments(self):
        selection = {'root': True, 'engine': True, 'browser': False}
        with patch.object(verification.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as run:
            verification.run_selected_checks(selection, offline=False, cargo_timings=True)
        commands = [call.args[0] for call in run.call_args_list if 'cargo' in call.args[0]]
        self.assertEqual(len(commands), 4)
        self.assertEqual([next(c for c in command if c in ('build', 'test')) for command in commands],
                         ['test', 'build', 'build', 'test'])
        for command in commands:
            self.assertEqual(command.count('--timings'), 1)
            self.assertIn('--locked', command)
            self.assertIn('-j', command)
            if 'test' in command:
                self.assertLess(command.index('--timings'), command.index('--'))
                self.assertIn('--test-threads=2', command)

    def test_overlap_root_build_opt_in_and_focused_remains_sequential(self):
        for focused in ([], ['capture::']):
            with self.subTest(focused=focused), patch.object(verification, 'overlap_cargo_test_and_build') as overlap, patch.object(
                    verification.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, stdout='test result: ok. 1 passed; 0 failed\n')) as run:
                verification.run_selected_checks({'root': True, 'engine': False, 'focused': focused},
                                                  overlap_root_build=True)
                self.assertEqual(overlap.called, not focused)
                if not focused:
                    test_command, build_command = overlap.call_args.args
                    self.assertIn('--message-format=json-render-diagnostics', test_command)
                    self.assertEqual(test_command[-2:], ['--', '--test-threads=2'])
                    self.assertIn('build', build_command)
                else:
                    self.assertNotIn('--message-format=json-render-diagnostics', run.call_args.args[0])

    def test_overlap_root_build_uses_event_gates_and_cleans_processes(self):
        # The harness asserts the build has not begun before the event, then
        # waits on a build-owned marker to prove the two workloads overlap.
        real_spawn = subprocess.Popen
        with tempfile.TemporaryDirectory() as tmp:
            marker = Path(tmp) / 'build-started'
            build_started = Path(tmp) / 'build-started'
            build_done = Path(tmp) / 'build-done'
            test_script = (
                'import json,os,sys,time; '
                'print(json.dumps({"reason":"compiler-artifact"}),flush=True); '
                'print(json.dumps({"reason":"compiler-message","message":{"rendered":"warning: visible\\n"}}),flush=True); '
                'assert not os.path.exists(sys.argv[2]); '
                'print(json.dumps({"reason":"build-finished","success":True}),flush=True); '
                'deadline=time.monotonic()+5; '
                'exec("while not os.path.exists(sys.argv[3]):\\n if time.monotonic()>deadline: raise SystemExit(4)\\n time.sleep(.01)")')
            build_script = 'import sys; open(sys.argv[1],"w").close(); open(sys.argv[2],"w").close()'
            output = []
            with patch.object(verification.subprocess, 'Popen', side_effect=lambda cmd, **kw: real_spawn(cmd, **kw)), patch.object(
                    sys, 'stdout', type('Out', (), {'write': lambda _s, x: output.append(x), 'flush': lambda _s: None})()):
                verification.overlap_cargo_test_and_build(
                    [sys.executable, '-c', test_script, str(marker), str(build_started), str(build_done)],
                    [sys.executable, '-c', build_script, str(build_started), str(build_done)])
            self.assertTrue(marker.exists())
            self.assertTrue(build_started.exists())
            self.assertTrue(build_done.exists())
            self.assertIn('warning: visible', ''.join(output))
            self.assertNotIn('compiler-artifact', ''.join(output))

    def test_overlap_root_build_fails_closed_on_false_or_missing_event(self):
        for output in ('{"reason":"build-finished","success":false}\n',
                       '{"reason":"compiler-artifact"}\n', 'plain output\n'):
            script = 'import sys; sys.stdout.write(sys.argv[1]); sys.stdout.flush()'
            build = [sys.executable, '-c', 'raise SystemExit(9)']
            with self.subTest(output=output):
                with self.assertRaises(RuntimeError), patch.object(verification.subprocess, 'Popen', wraps=subprocess.Popen) as spawn:
                    verification.overlap_cargo_test_and_build([sys.executable, '-c', script, output], build)
                self.assertEqual(spawn.call_count, 1)

    def test_overlap_root_build_reaps_test_and_build_failures(self):
        real_spawn = subprocess.Popen
        gate = '{"reason":"build-finished","success":true}\n'
        for mode in ('test-failure', 'build-failure'):
            processes = []
            def spawn(command, **kwargs):
                process = real_spawn(command, **kwargs)
                processes.append(process)
                return process
            if mode == 'test-failure':
                started = Path(tempfile.gettempdir()) / f'overlap-started-{os.getpid()}'
                try: started.unlink()
                except FileNotFoundError: pass
                self.addCleanup(started.unlink, missing_ok=True)
                test = [sys.executable, '-c',
                        'import os,sys,time; print(sys.argv[1],flush=True); deadline=time.monotonic()+5; '
                        'exec("while not os.path.exists(sys.argv[2]):\\n if time.monotonic()>deadline: raise SystemExit(5)\\n time.sleep(.01)"); '
                        'print("test failed",flush=True); raise SystemExit(8)', gate, str(started)]
                build = [sys.executable, '-c', 'import sys; open(sys.argv[1],"w").close()', str(started)]
            else:
                test = [sys.executable, '-c', 'import sys,time; print(sys.argv[1],flush=True); time.sleep(10)', gate]
                build = [sys.executable, '-c', 'raise SystemExit(9)']
            with self.subTest(mode=mode), patch.object(verification.subprocess, 'Popen', side_effect=spawn):
                with self.assertRaises(subprocess.CalledProcessError) as failure:
                    verification.overlap_cargo_test_and_build(test, build)
            self.assertEqual(failure.exception.returncode, 8 if mode == 'test-failure' else 9)
            self.assertTrue(processes)
            self.assertTrue(all(process.poll() is not None for process in processes))

    def test_overlap_root_build_spawn_failure_and_spawn_signal_restore_handlers(self):
        previous = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT)}
        with patch.object(verification.subprocess, 'Popen', side_effect=OSError('spawn failed')):
            with self.assertRaisesRegex(OSError, 'spawn failed'):
                verification.overlap_cargo_test_and_build(['never'], ['never'])
        self.assertEqual({sig: signal.getsignal(sig) for sig in previous}, previous)
        real_spawn = subprocess.Popen
        processes = []
        def interrupted_spawn(command, **kwargs):
            process = real_spawn(command, **kwargs)
            processes.append(process)
            os.kill(os.getpid(), signal.SIGTERM)
            return process
        with patch.object(verification.subprocess, 'Popen', side_effect=interrupted_spawn):
            with self.assertRaises(SystemExit) as error:
                verification.overlap_cargo_test_and_build([sys.executable, '-c', 'import time; time.sleep(10)'], ['never'])
        self.assertEqual(error.exception.code, 128 + signal.SIGTERM)
        self.assertTrue(all(process.poll() is not None for process in processes))
        self.assertEqual({sig: signal.getsignal(sig) for sig in previous}, previous)

    def test_overlap_root_build_second_spawn_failure_reaps_test(self):
        real_spawn = subprocess.Popen
        processes = []
        def spawn(command, **kwargs):
            if processes:
                raise OSError('app build spawn failed')
            process = real_spawn(command, **kwargs)
            processes.append(process)
            return process
        cargo = [sys.executable, '-c',
                 'import sys,time; print(sys.argv[1],flush=True); time.sleep(10)',
                 '{"reason":"build-finished","success":true}\n']
        with patch.object(verification.subprocess, 'Popen', side_effect=spawn):
            with self.assertRaisesRegex(OSError, 'app build spawn failed'):
                verification.overlap_cargo_test_and_build(cargo, ['never'])
        self.assertEqual(len(processes), 1)
        self.assertIsNotNone(processes[0].poll())

    def test_overlap_root_build_notices_failure_with_partial_line_and_signal_during_build_tail(self):
        real_spawn = subprocess.Popen
        gate = '{"reason":"build-finished","success":true}\n'
        # An app-build failure is noticed even when the test pipe has no newline.
        test = [sys.executable, '-c',
                'import sys,time; print(sys.argv[1],flush=True); sys.stdout.write("partial diagnostic"); sys.stdout.flush(); time.sleep(10)', gate]
        build = [sys.executable, '-c', 'raise SystemExit(11)']
        processes = []
        def spawn(command, **kwargs):
            process = real_spawn(command, **kwargs)
            processes.append(process)
            return process
        output = []
        with patch.object(verification.subprocess, 'Popen', side_effect=spawn), patch.object(
                sys, 'stdout', type('Out', (), {'write': lambda _s, x: output.append(x), 'flush': lambda _s: None})()):
            with self.assertRaises(subprocess.CalledProcessError) as failure:
                verification.overlap_cargo_test_and_build(test, build)
        self.assertEqual(failure.exception.returncode, 11)
        self.assertIn('partial diagnostic', ''.join(output))
        self.assertTrue(all(process.poll() is not None for process in processes))

        # The harness exits, leaving only the app build active; SIGTERM still
        # interrupts that tail and reaps its owned process group.
        processes.clear()
        test = [sys.executable, '-c', 'import sys; print(sys.argv[1],flush=True)', gate]
        build = [sys.executable, '-c', 'import os,sys,time; os.kill(os.getppid(),15); time.sleep(10)']
        with patch.object(verification.subprocess, 'Popen', side_effect=spawn):
            with self.assertRaises(SystemExit) as error:
                verification.overlap_cargo_test_and_build(test, build)
        self.assertEqual(error.exception.code, 128 + signal.SIGTERM)
        self.assertTrue(all(process.poll() is not None for process in processes))

    def test_root_test_and_fresh_app_build_failures_stop_verification(self):
        for focused in ([], ['capture::']):
            for failure in ('test', 'build'):
                commands = []
                def execute(command, **kwargs):
                    if 'cargo' in command:
                        commands.append(command)
                        self.assertTrue(kwargs['check'])
                        if failure in command:
                            raise subprocess.CalledProcessError(7, command)
                    return subprocess.CompletedProcess(command, 0, stdout='test result: ok. 2 passed; 0 failed\n')
                with self.subTest(focused=focused, failure=failure), patch.object(
                        verification.subprocess, 'run', side_effect=execute):
                    with self.assertRaises(subprocess.CalledProcessError):
                        verification.run_selected_checks({'root': True, 'engine': True,
                                                          'browser': False, 'focused': focused})
                self.assertEqual(len(commands), 1 if failure == 'test' else 2)
                self.assertIn('test', commands[0])
                if failure == 'build':
                    self.assertIn('build', commands[1])
                self.assertFalse(any('capture/engine/Cargo.toml' in c for c in commands))

    def test_explicit_platform_suites_execute_even_when_changed_tree_has_no_matching_inputs(self):
        for suite in ('rust', 'browser'):
            args = ['verification.py', '--base', 'unknown', '--run', '--suite', suite]
            with self.subTest(suite=suite), patch.object(verification.sys, 'argv', args), patch.object(
                    verification, 'plan', return_value=verification.select(['README.md'])), patch.object(
                    verification.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0)) as run:
                verification.main()
                commands = [call.args[0] for call in run.call_args_list]
                self.assertEqual(['node', 'scripts/test-browser.mjs'] in commands, suite == 'browser')
                self.assertEqual(any('cargo' in command for command in commands), suite == 'rust')

    def test_engine_tests_compile_and_test_package(self):
        result = verification.select(["capture/engine/tests/process.rs"])
        self.assertFalse(result["root"])
        self.assertTrue(result["engine"])

    def test_unknown_shared_build_security_lifecycle_and_tooling_broaden(self):
        for path in ["new-owner/foo.rs", "Cargo.toml", "Cargo.lock", "build.rs",
                     "assets/icon.png", "src/ui/recording_completion.rs", "src/handoff.rs",
                     "capture/engine/src/session.rs", "capture/desktop/native/Settings.swift",
                     "enablement/verification.py", ".githooks/pre-push",
                     ".github/workflows/verification.yml", "public/site/package.json", "connect/service/src/auth.ts"]:
            with self.subTest(path=path):
                result = verification.select([path])
                self.assertTrue(result["root"])
                self.assertTrue(result["engine"])

    def test_mixed_delta_cannot_hide_broad_path(self):
        self.assertTrue(verification.select(["README.md", "main.rs"])["root"])

    def test_empty_or_missing_history_is_broad(self):
        self.assertTrue(verification.select([])["root"])
        self.assertTrue(verification.plan("nonexistent-base", "HEAD")["root"])

    def test_deterministic_union(self):
        self.assertEqual(verification.select(["README.md", "main.rs", "README.md"]),
                         verification.select(["main.rs", "README.md"]))

    def test_independent_browser_selects_its_existing_build_and_fixtures(self):
        result = verification.select(["connect/web/src/Hosted.tsx", "connect/web/package.json",
                                      "connect/web/test/closure.test.mjs", "connect/web/test/routes.test.mjs"])
        self.assertFalse(result["root"])
        self.assertFalse(result["engine"])
        self.assertTrue(result["browser"])
        for path in ["connect/web/unknown.rs", "connect/service/src/auth.ts", "build.rs"]:
            with self.subTest(path=path):
                self.assertTrue(verification.select(["connect/web/src/api.ts", path])["root"])

    def test_selected_absent_browser_package_fails_closed(self):
        real_run = subprocess.run
        def execute(command, **kwargs):
            if command[0] in ("python3", "sh"):
                return subprocess.CompletedProcess(command, 0)
            return real_run(command, **kwargs)
        with tempfile.TemporaryDirectory() as tmp, patch.object(verification, "ROOT", Path(tmp)), patch.object(
                verification.subprocess, "run", side_effect=execute):
            with self.assertRaises(FileNotFoundError) as failure:
                verification.run_checks(verification.select(["connect/web/package.json"]))
            self.assertEqual(str(failure.exception.filename), str(Path(tmp) / "connect/web"))

    def test_present_browser_package_missing_real_driver_fails(self):
        real_run = subprocess.run
        def execute(command, **kwargs):
            # Isolate installation/compilation, but execute the selected driver
            # with real Node: absent test coverage must not become a green lane.
            if command == ["node", "scripts/test-browser.mjs"]:
                return real_run(command, **kwargs, capture_output=True)
            return subprocess.CompletedProcess(command, 0)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "connect/web").mkdir(parents=True)
            with patch.object(verification, "ROOT", root), patch.object(
                    verification.subprocess, "run", side_effect=execute):
                with self.assertRaises(subprocess.CalledProcessError) as failure:
                    verification.run_checks(verification.select(["connect/web/src/SharedLinks.tsx"]))
                self.assertIn(b"MODULE_NOT_FOUND", failure.exception.stderr)

    def test_shared_links_is_exact_browser_inventory_not_shared_asset_permission(self):
        selected = verification.select(["connect/web/src/SharedLinks.tsx"])
        self.assertEqual((selected["root"], selected["engine"], selected["browser"]),
                         (False, False, True))
        for path in ["connect/web/src/Other.tsx", "connect/web/src/sharedlinks.tsx",
                     "public/site/scripts/cd/publish_pages.py", "public/site/public/hosted/source-pond-v4.png",
                     "public/site/public/hosted/agent-pond-v4.png", "public/site/public/hosted/agent-reading-v4.png",
                     "public/site/public/HostGrotesk-Variable.ttf", "public/site/public/apparent-wordmark.svg"]:
            with self.subTest(path=path):
                result = verification.select(["connect/web/src/SharedLinks.tsx", path])
                self.assertTrue(result["root"])
                self.assertTrue(result["engine"])
                self.assertTrue(result["browser"])

    def test_reviewed_moves_require_complete_exact_source_pairs(self):
        # Real immutable Git blobs, not a caller-supplied mechanical label.
        for name, recipe in verification.MOVES.items():
            with self.subTest(name=name):
                before = {p: ("100644", pair[0]) if pair[0] else None for p, pair in recipe["blobs"].items()}
                after = {p: ("100644", pair[1]) for p, pair in recipe["blobs"].items()}
                def tree(rev, path):
                    return (before if rev == "base" else after).get(path)
                with patch.object(verification, "tree_entry", side_effect=tree), patch.object(
                        verification, "changed_paths", return_value=list(after)):
                    result = verification.plan("base", "head")
                    self.assertTrue(result["root"])
                    self.assertFalse(result["engine"])
                    self.assertEqual(result["focused"], recipe["tests"])
                    # A changed security/lifecycle body, test deletion, executable
                    # mode or symlink must never retain the focused route.
                    for path in after:
                        original = after[path]
                        for altered in [("100644", "f" * 40), None, ("100755", original[1]), ("120000", original[1])]:
                            after[path] = altered
                            self.assertTrue(verification.plan("base", "head")["engine"])
                        after[path] = original
                with patch.object(verification, "tree_entry", side_effect=tree), patch.object(
                        verification, "changed_paths", return_value=[*after, "Cargo.lock"]):
                    self.assertTrue(verification.plan("base", "head")["engine"])

    def test_focused_checks_compile_root_and_reject_zero_matches(self):
        result = subprocess.CompletedProcess([], 0, stdout="test result: ok. 2 passed; 0 failed\n")
        selection = {"root": True, "engine": False, "browser": False, "focused": ["capture::"]}
        with patch.object(verification.subprocess, "run", return_value=result) as run:
            verification.run_checks(selection)
            commands = [call.args[0] for call in run.call_args_list]
            self.assertTrue(any("build" in c for c in commands))
            self.assertTrue(any("capture::" in c for c in commands))
            self.assertFalse(any("capture/engine/Cargo.toml" in c for c in commands))
            self.assertFalse(any("test" in c and "capture::" not in c for c in commands))
        result.stdout = "test result: ok. 0 passed; 0 failed; 90 filtered out\n"
        with patch.object(verification.subprocess, "run", return_value=result):
            with self.assertRaises(RuntimeError):
                verification.run_checks(selection)

    def test_retired_context_deletion_is_document_only_not_unknown_markdown(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, env=env, text=True).strip()
            git("init", "-q")
            git("config", "user.name", "Test")
            git("config", "user.email", "test@example.invalid")
            (root / "CONTEXT.md").write_text("Retired glossary\n")
            (root / "docs").mkdir()
            (root / "docs/routing.md").write_text("See CONTEXT.md\n")
            git("add", ".")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD")
            git("rm", "CONTEXT.md")
            (root / "docs/routing.md").write_text("Historical routing\n")
            git("add", ".")
            git("commit", "-qm", "delete retired glossary")
            with patch.object(verification, "ROOT", root), patch.dict(os.environ, env, clear=True):
                result = verification.plan(base, "HEAD")
            self.assertEqual(result["paths"], ["CONTEXT.md", "docs/routing.md"])
            self.assertFalse(result["root"])
            self.assertFalse(result["engine"])
            self.assertFalse(result["browser"])
            self.assertTrue(verification.select(["UNKNOWN.md"])["engine"])
            self.assertTrue(verification.select([*result["paths"], "build.rs"])["engine"])

    def test_real_rename_and_delete_include_old_owner(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            # A real pre-push inherits GIT_DIR for its own repository. The
            # fixture must isolate both its setup and the selector subprocess.
            env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, env=env, text=True).strip()
            git("init", "-q")
            git("config", "user.name", "Test")
            git("config", "user.email", "test@example.invalid")
            (root / "source.rs").write_text("original")
            git("add", ".")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD")
            git("mv", "source.rs", "README.md")
            git("commit", "-qm", "rename")
            with patch.object(verification, "ROOT", root), patch.dict(os.environ, env, clear=True):
                result = verification.plan(base, "HEAD")
                self.assertEqual(result["paths"], ["README.md", "source.rs"])
                self.assertTrue(result["root"])
                git("rm", "README.md")
                git("commit", "-qm", "delete")
                self.assertEqual(verification.changed_paths(base, "HEAD"), ["source.rs"])

    def test_real_git_preservation_gate_and_shared_delta(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
            def git(*args):
                return subprocess.check_output(["git", *args], cwd=root, env=env, text=True).strip()
            git("init", "-q")
            git("config", "user.name", "Test")
            git("config", "user.email", "test@example.invalid")
            (root / "donor.rs").write_text("fn preserved() {}\n")
            git("add", ".")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD")
            old = git("rev-parse", "HEAD:donor.rs")
            (root / "moved.rs").write_text("fn preserved() {}\n")
            (root / "donor.rs").write_text("mod moved;\n")
            git("add", ".")
            git("commit", "-qm", "move")
            recipe = {"fixture": {"blobs": {
                "donor.rs": (old, git("rev-parse", "HEAD:donor.rs")),
                "moved.rs": (None, git("rev-parse", "HEAD:moved.rs")),
            }, "tests": ["preserved::"]}}
            with patch.object(verification, "ROOT", root), patch.object(verification, "MOVES", recipe), patch.dict(os.environ, env, clear=True):
                self.assertEqual(verification.plan(base, "HEAD")["focused"], ["preserved::"])
                (root / "Cargo.lock").write_text("shared dependency change\n")
                git("add", ".")
                git("commit", "-qm", "shared input")
                self.assertTrue(verification.plan(base, "HEAD")["engine"])
                self.assertEqual(verification.plan(base, "HEAD")["focused"], [])

    def test_real_git_every_recipe_pins_both_trees_paths_modes_and_blobs(self):
        # Build real trees with the production registry's exact blob IDs.
        # --missing-ok permits historical blobs absent from shallow checkouts:
        # diff --no-renames/ls-tree read tree metadata, not source bodies. This
        # proves routing pins, not source preservation (reviewed separately).
        # Independent fixed acceptance fixtures catch deletion/drift of either
        # newly supported pair, not just mutations of a registry-derived tree.
        current_pairs = {
            "config-d59796c3-e86f515e": {
                "blobs": {
                    "src/handoff.rs": ("da2980adcf8e50b35019b043059d26c1697c8dd7", "7dcdf6b051b10d45d0d2e1afbfac5a5dc12ae393"),
                    "src/handoff/config_store.rs": (None, "4d960d819900123d099aa6ad4d2a7827e3ffbfdc"),
                    "src/handoff/test_modules.rs": (None, "9ac7c770ecb3aeb2d86892b5656330cf5292c405"),
                },
                "tests": ["handoff::", "hosted::", "settings::"],
            },
            "processing-d59796c3-f135448c": {
                "blobs": {
                    "main.rs": ("b54efd65ae545d971c964b51b77fff7b2d4a3711", "7e8656f9e257d5fae635a47bdc7171c08604fca4"),
                    "src/bundle_processing.rs": (None, "88240b2b938639cfbdd190ad1209e5f306ac3f33"),
                },
                "tests": ["bundle_processing::", "capture::", "recover::", "sketch::"],
            },
            # Recorder test topology repair: preserve the historical source
            # donor pair while pinning the current nested ordinary test module.
            "recorder-current": {
                "blobs": {
                    "src/capture.rs": ("6d26953bf949d238ace1bf292801e1bc218875a9", "1f607ee1b7e20fb36f0801c18551fa58570165f2"),
                    "capture/engine/runtime/recording/recorder.rs": (None, "a38a3c3c67ce4d01123a2578cdad14a2a7dddb8d"),
                    "capture/engine/runtime/recording/recorder/tests.rs": (None, "7868cef5ac5073562b5bbefed2392606fdc555dc"),
                },
                "tests": ["agent_capture::", "capture::"],
            },
        }
        for name, reviewed in {**verification.MOVES, **current_pairs}.items():
            with self.subTest(recipe=name), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
                def git(*args, input=None):
                    return subprocess.check_output(["git", *args], cwd=root, env=env,
                                                   text=True, input=input).strip()
                git("init", "-q")
                git("config", "user.name", "Test")
                git("config", "user.email", "test@example.invalid")
                git("read-tree", "--empty")
                for path, (old, _) in reviewed["blobs"].items():
                    if old:
                        git("update-index", "--add", "--cacheinfo", "100644", old, path)
                def commit():
                    return git("commit-tree", git("write-tree", "--missing-ok"), "-m", "fixture")
                base = commit()
                blobs = reviewed["blobs"]
                for path, (_, new) in blobs.items():
                    git("update-index", "--add", "--cacheinfo", "100644", new, path)
                head = commit()
                with patch.object(verification, "ROOT", root), patch.dict(os.environ, env, clear=True):
                    good = verification.plan(base, head)
                    self.assertTrue(good["root"])
                    self.assertFalse(good["engine"])
                    self.assertEqual(good["focused"], sorted(reviewed["tests"]))
                    def broad(left, right):
                        result = verification.plan(left, right)
                        self.assertTrue(result["root"])
                        self.assertTrue(result["engine"])
                        self.assertEqual(result["focused"], [])
                    # Mutate either endpoint independently, including donor
                    # modes and destinations that must not exist at the base.
                    for revision, is_base in [(base, True), (head, False)]:
                        for path, (old, _) in blobs.items():
                            modes = ["100644", "100755", "120000"] if not is_base or old else ["100644"]
                            for mode in modes:
                                git("read-tree", revision)
                                oid = git("hash-object", "-w", "--stdin", input="altered\n") if mode == "100644" else (old if is_base else blobs[path][1])
                                git("update-index", "--add", "--cacheinfo", mode, oid, path)
                                tree = git("write-tree", "--missing-ok")
                                changed = git("commit-tree", tree, "-m", "mutation")
                                broad(changed if is_base else base, head if is_base else changed)
                            if not is_base:
                                git("read-tree", head)
                                git("update-index", "--force-remove", path)
                                changed = commit()
                                broad(base, changed)
                    for path in ["Cargo.lock", "build.rs", "src/unknown.rs",
                                 "enablement/tests/test_verification.py", "public/site/scripts/cd/publish_pages.py"]:
                        git("read-tree", head)
                        oid = git("hash-object", "-w", "--stdin", input="mixed input\n")
                        git("update-index", "--add", "--cacheinfo", "100644", oid, path)
                        changed = commit()
                        broad(base, changed)
                    broad("missing-history", head)
                    # A document alongside an exact move remains supported.
                    git("read-tree", head)
                    git("update-index", "--add", "--cacheinfo", "100644", oid, "README.md")
                    documented = commit()
                    self.assertEqual(verification.plan(base, documented)["focused"], sorted(reviewed["tests"]))


class RuntimeBudgetTests(unittest.TestCase):
    def test_budget_preserves_status_and_stdin(self):
        result = subprocess.run(['python3', str(SCRIPT), '--hook-budget', '2',
                                 'sh', '-c', 'cat; exit 7'], input='refs\n',
                                text=True, capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 7)
        self.assertEqual(result.stdout, 'refs\n')

    def test_budget_kills_descendants_and_assigns_optimization(self):
        # Inherited pipes stay open if the descendant escapes cleanup.
        result = subprocess.run(['python3', str(SCRIPT), '--hook-budget', '.1',
                                 'sh', '-c', 'sleep 30 & wait'],
                                text=True, capture_output=True, timeout=5)
        self.assertEqual(result.returncode, 124)
        self.assertIn('Optimize the tests', result.stderr)
        self.assertIn('agent whose run hit this limit owns', result.stderr)

    def test_cancelled_watchdog_cleans_up_process_group(self):
        import select
        import signal
        # Deliver a real OS signal after Popen creates the child but before the
        # watchdog owns its return value. Avoid an unrelated stdin rendezvous
        # in this cancellation window. The running case still signals externally.
        driver = '''
import os, runpy, signal, subprocess, sys
watchdog = runpy.run_path(sys.argv[1])['hook_budget']
spawn = subprocess.Popen
def held_spawn(*args, **kwargs):
    child = spawn(*args, **kwargs)
    print(child.pid, flush=True)
    sig = int(sys.argv[3])
    handler = signal.getsignal(sig)
    observed = []
    def record(signum, frame):
        observed.append(signum)
        return handler(signum, frame)
    signal.signal(sig, record)
    os.kill(os.getpid(), sig)
    assert observed == [sig], 'signal must run before Popen returns'
    return child
if sys.argv[2] == 'spawn':
    subprocess.Popen = held_spawn
    command = 'sleep 30 & wait'
else:
    command = 'sleep 30 & echo $$; wait'
sys.exit(watchdog(30, ['sh', '-c', command]))
'''
        for phase in ('spawn', 'running'):
            for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
                with self.subTest(phase=phase, signal=sig):
                    child_group = None
                    with subprocess.Popen([sys.executable, '-c', driver, str(SCRIPT), phase, str(sig)],
                                          text=True, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                          stderr=subprocess.PIPE, start_new_session=True) as process:
                        try:
                            self.assertTrue(select.select([process.stdout], [], [], 5)[0], 'child did not start')
                            child_group = int(process.stdout.readline())
                            if phase == 'running':
                                process.send_signal(sig)
                            # EOF must include the whole child group, not just
                            # the watchdog. Keep the original five-second bound.
                            stdout, stderr = process.communicate(timeout=5)
                            self.assertEqual(process.returncode, 128 + sig, stderr)
                            self.assertEqual(stdout, '')
                        finally:
                            # Bound failure cleanup even when the watchdog has
                            # exited but descendants still hold captured pipes.
                            if child_group is not None:
                                try: os.killpg(child_group, signal.SIGKILL)
                                except ProcessLookupError: pass
                            if process.poll() is None:
                                process.kill()
                            process.communicate(timeout=5)

    def test_watchdog_restores_handlers_when_spawn_fails(self):
        import signal
        previous = {sig: signal.getsignal(sig) for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT)}
        with patch.object(verification.subprocess, 'Popen', side_effect=OSError('synthetic launch failure')):
            with self.assertRaisesRegex(OSError, 'synthetic launch failure'):
                verification.hook_budget(30, ['synthetic-command'])
        self.assertEqual({sig: signal.getsignal(sig) for sig in previous}, previous)

    def test_supported_ci_jobs_have_platform_execution_budget(self):
        import re
        for workflow in (SCRIPT.parents[1] / '.github/workflows').glob('*.yml'):
            if workflow.name == 'release-windows.yml':
                continue  # Retired platform is outside the supported CI contract.
            for job in re.split(r'^  [\w-]+:\n', workflow.read_text(), flags=re.M)[1:]:
                if re.search(r'^    runs-on:', job, re.M):
                    expected = ("${{ startsWith(inputs.runner, 'macos-') && 15 || 10 }}"
                                if workflow.name == 'verification-platform.yml' else
                                '15' if workflow.name in {'verification-mac.yml', 'native-proof-mac.yml', 'release-mac-preview.yml'} else '10')
                    self.assertIn(f'    timeout-minutes: {expected}\n', job, str(workflow))
                    if '    name: Workflow runtime budget\n' not in job:
                        self.assertIn('agent whose run hits the limit owns optimization', job)


if __name__ == "__main__":
    unittest.main()
