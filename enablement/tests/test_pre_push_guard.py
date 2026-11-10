"""Exercise the actual pre-push hook in disposable Git repositories."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


HOOK = Path(__file__).resolve().parents[2] / ".githooks" / "pre-push"
SELECTOR = HOOK.parents[1] / "enablement" / "verification.py"


def run(*args, cwd, env=None, input=None):
    # Git exports its own GIT_DIR/GIT_WORK_TREE when it invokes a hook. Never
    # let those redirect fixture commands back to the checkout being pushed.
    clean_env = {key: value for key, value in (env or os.environ).items()
                 if not key.startswith("GIT_")}
    return subprocess.run(args, cwd=cwd, env=clean_env, input=input, text=True,
                          capture_output=True, check=False)


class PrePushGuardTests(unittest.TestCase):
    def setUp(self):
        # A dedicated short, canonical fixture job root is independent of both
        # CI's long TMPDIR and an untrusted caller TMPDIR. It also keeps Clanker
        # fixtures in private apparent-dev jobs rather than arbitrary /tmp names.
        self.tmp = tempfile.TemporaryDirectory(prefix="apparent-dev-hook-", dir="/private/tmp" if sys.platform == "darwin" else "/tmp")
        self.addCleanup(self.tmp.cleanup)
        self.repo = Path(self.tmp.name) / "repo"
        self.repo.mkdir()
        run("git", "init", "-q", str(self.repo), cwd=self.repo)
        run("git", "config", "user.name", "Test", cwd=self.repo)
        run("git", "config", "user.email", "test@example.invalid", cwd=self.repo)
        (self.repo / "file").write_text("original\n")
        (self.repo / "enablement").mkdir()
        (self.repo / "enablement" / "tests").mkdir()
        # The driver runs tests as well as Cargo. Keep this fixture finite:
        # copying the real hook tests here would recursively invoke itself.
        fixture_test = "import unittest\nclass Fixture(unittest.TestCase):\n    def test_fixture(self):\n        self.assertTrue(True)\n"
        for name in ("test_fixture_push.py", "test_verification.py", "test_release_gate.py",
                     "test_temp_preflight.py", "test_owner_lanes.py"):
            (self.repo / "enablement" / "tests" / name).write_text(fixture_test)
        shutil.copy2(SELECTOR, self.repo / "enablement" / "verification.py")
        local_tests = self.repo / "enablement/local-tests"
        local_tests.mkdir()
        shutil.copy2(SELECTOR.parent / "local-tests/run.ts", local_tests / "run.ts")
        for name in ("service.ts", "receipt.ts"):
            shutil.copy2(SELECTOR.parent / "local-tests" / name, local_tests / name)
        # Finite TS contract fixtures, just like the Python fixtures above.
        for name in ("local-tests/run.test.ts", "local-tests/reuse.test.ts", "prerequisites/native.test.ts", "timing/command.test.ts", "timing/mac-cache.test.ts", "ci/select.test.ts"):
            path = self.repo / "enablement" / name
            path.parent.mkdir(exist_ok=True)
            path.write_text("import {test} from 'node:test'; test('fixture', () => {});\n")
        for name in ("hermes/desktop/controller.test.ts", "hermes/desktop/plugin.test.ts",
                     "capture/desktop/ui/frontend/src/hermes/protocol.test.ts",
                     "capture/desktop/ui/frontend/src/hermes/mailbox.test.ts"):
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("import {test} from 'node:test'; test('fixture', () => {});\n")
        for name in ("inputs.ts", "workflow-controls.ts"):
            shutil.copy2(SELECTOR.parent / "ci" / name, self.repo / "enablement/ci" / name)
        # The real verifier now runs the shared frontend color guard. Preserve
        # its actual dependency closure in this disposable hook repository.
        theme = self.repo / "shared/ui/scripts"
        theme.mkdir(parents=True)
        for name in ("check-theme.py", "legacy-colors.json"):
            shutil.copy2(SELECTOR.parents[1] / "shared/ui/scripts" / name, theme / name)
        run("git", "add", ".", cwd=self.repo)
        run("git", "commit", "-qm", "original", cwd=self.repo)
        self.head = run("git", "rev-parse", "HEAD", cwd=self.repo).stdout.strip()
        self.bin = Path(self.tmp.name) / "bin"
        self.bin.mkdir()
        self.log = Path(self.tmp.name) / "commands"
        taskpolicy = self.bin / "taskpolicy"
        taskpolicy.write_text("#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$COMMAND_LOG\"\nexit \"${MOCK_STATUS:-0}\"\n")
        taskpolicy.chmod(0o755)
        self.env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}",
                        COMMAND_LOG=str(self.log), PYTHONDONTWRITEBYTECODE="1", TMPDIR=self.tmp.name)
        # Default-policy fixtures must not inherit the live build runner's
        # allocation. Exercise explicit allocations separately below.
        self.env.pop("T3_BUILD_JOBS", None)
        # Complete owner-level fallback is intentional for unknown history.
        # Stub expensive package tools, retaining the real selector and guards.
        for directory in ("connect/client", "connect/service", "connect/web", "public/site",
                          "capture/desktop/ui/frontend", "enablement/directory-width/tests", "hermes/tests"):
            (self.repo / directory).mkdir(parents=True, exist_ok=True)
        # Cargo is stubbed in this hook fixture, so the production-binary check
        # needs a finite TS fixture too. Its real behavior has its own CLI test.
        encryption_test = self.repo / "connect/client/tests/encryption-default.test.ts"
        encryption_test.parent.mkdir()
        encryption_test.write_text("import {test} from 'node:test'; test('finite production CLI fixture', () => {});\n")
        (self.repo / "enablement/cli-version.test.ts").write_text(encryption_test.read_text())
        # Fixture directories must survive the commit/clean-tree guard.
        for directory in ("enablement/directory-width/tests", "hermes/tests"):
            (self.repo / directory / "test_fixture.py").write_text(fixture_test)
        for tool in ("npm", "bun", "cargo", "xvfb-run"):
            stub = self.bin / tool
            stub.write_text('#!/bin/sh\nexit "${MOCK_STATUS:-0}"\n')
            stub.chmod(0o755)
        # Root/Engine run through the taskpolicy stub; Linux's independently
        # selected client invokes Cargo directly. Observe that allocation too.
        (self.bin / "cargo").write_text('#!/bin/sh\nprintf "cargo %s\\n" "$*" >> "$COMMAND_LOG"\nexit "${MOCK_STATUS:-0}"\n')
        node = self.bin / "node"
        node.write_text('#!/bin/sh\ncase "$1" in scripts/site-build/verify.mjs|scripts/test-browser.mjs) exit "${MOCK_STATUS:-0}";; esac\nexec ' + shutil.which("node") + ' "$@"\n')
        node.chmod(0o755)
        run("git", "add", ".", cwd=self.repo)
        run("git", "commit", "-qm", "finite owner fixtures", cwd=self.repo)
        self.head = run("git", "rev-parse", "HEAD", cwd=self.repo).stdout.strip()

    def push(self, oid=None, local="refs/heads/feature", remote="refs/heads/main", env=None):
        line = f"{local} {oid or self.head} {remote} {'0' * 40}\n"
        return run("sh", str(HOOK), "origin", "example.invalid:repo", cwd=self.repo,
                   env=env or self.env, input=line)

    def test_fixture_commands_ignore_parent_hook_git_environment(self):
        env = dict(self.env, GIT_DIR=str(HOOK.parents[1] / ".git"),
                   GIT_WORK_TREE=str(HOOK.parents[1]))
        result = run("git", "rev-parse", "--show-toplevel", cwd=self.repo, env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), str(self.repo))

    @unittest.skipIf(sys.platform == "darwin", "Mac fixtures allocate their canonical short root separately")
    def test_ci_length_temp_root_does_not_overflow_nested_verification(self):
        with tempfile.TemporaryDirectory(prefix="apparent-dev-prepush-ci-long-", dir="/tmp") as parent:
            env = dict(os.environ, TMPDIR=parent, PYTHONDONTWRITEBYTECODE="1")
            # Establish the parent is genuinely admitted, then execute the real
            # hook test below it. The previous extra nesting rejected 113 bytes.
            admitted = run("python3", str(SELECTOR), "--base", "unknown", "--preflight",
                           cwd=HOOK.parents[1], env=env)
            self.assertEqual(admitted.returncode, 0, admitted.stderr)
            result = run("python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                         "-p", "test_pre_push_guard.py", "-k", "test_clean_head_requires_build_and_tests",
                         cwd=HOOK.parents[1], env=env)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Ran 1 test", result.stderr)

    def test_contract_fixture_does_not_admit_the_callers_untrusted_tmpdir(self):
        parent = Path(self.tmp.name) / "untrusted-parent"
        parent.mkdir(mode=0o777)
        parent.chmod(0o777)
        result = run("python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                     "-p", "test_pre_push_guard.py", "-k", "test_clean_head_requires_build_and_tests",
                     cwd=HOOK.parents[1], env=dict(os.environ, TMPDIR=str(parent), PYTHONDONTWRITEBYTECODE="1"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Ran 1 test", result.stderr)

    def test_feature_push_runs_contracts_without_cargo(self):
        run("git", "update-ref", "refs/remotes/origin/main", self.head, cwd=self.repo)
        (self.repo / "README.md").write_text("docs\n")
        run("git", "add", "README.md", cwd=self.repo)
        run("git", "commit", "-qm", "docs", cwd=self.repo)
        self.head = run("git", "rev-parse", "HEAD", cwd=self.repo).stdout.strip()
        result = self.push(remote="refs/heads/feature")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.log.exists())

    def test_new_contract_filename_is_not_silently_omitted(self):
        run("git", "update-ref", "refs/remotes/origin/main", self.head, cwd=self.repo)
        (self.repo / "enablement/tests/test_new_feature.py").write_text(
            "import unittest\nclass NewFeature(unittest.TestCase):\n    def test_regression(self):\n        self.fail('new regression reached')\n")
        run("git", "add", ".", cwd=self.repo)
        run("git", "commit", "-qm", "new contract", cwd=self.repo)
        self.head = run("git", "rev-parse", "HEAD", cwd=self.repo).stdout.strip()
        result = self.push(remote="refs/heads/feature")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("new regression reached", result.stderr)
        self.assertIn("push rejected", result.stderr)

    def test_clean_head_requires_build_and_tests(self):
        result = self.push()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_text().splitlines(), [
            "-c utility cargo build -j 3 --offline",
            "-c utility cargo test -j 3 --offline -- --test-threads=2",
            "-c utility cargo build --manifest-path capture/engine/Cargo.toml -j 3 --offline",
            "-c utility cargo test --manifest-path capture/engine/Cargo.toml -j 3 --offline -- --test-threads=2",
            ("-c utility " if sys.platform == "darwin" else "") + "cargo build --manifest-path connect/client/Cargo.toml -j 3 --offline",
            ("-c utility " if sys.platform == "darwin" else "") + "cargo test --manifest-path connect/client/Cargo.toml -j 3 --offline -- --test-threads=2",
        ])

    def test_failed_verification_rejects_push(self):
        result = self.push(env=dict(self.env, MOCK_STATUS="1"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("failed", result.stderr)

    def test_explicit_build_allocation_reaches_all_cargo_commands(self):
        result = self.push(env=dict(self.env, T3_BUILD_JOBS="7"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_text().splitlines(), [
            "-c utility cargo build -j 7 --offline",
            "-c utility cargo test -j 7 --offline -- --test-threads=2",
            "-c utility cargo build --manifest-path capture/engine/Cargo.toml -j 7 --offline",
            "-c utility cargo test --manifest-path capture/engine/Cargo.toml -j 7 --offline -- --test-threads=2",
            ("-c utility " if sys.platform == "darwin" else "") + "cargo build --manifest-path connect/client/Cargo.toml -j 7 --offline",
            ("-c utility " if sys.platform == "darwin" else "") + "cargo test --manifest-path connect/client/Cargo.toml -j 7 --offline -- --test-threads=2",
        ])

    def test_document_only_push_avoids_cargo(self):
        run("git", "update-ref", "refs/remotes/origin/main", self.head, cwd=self.repo)
        (self.repo / "enablement/refactor.md").write_text("plan\n")
        run("git", "add", "enablement/refactor.md", cwd=self.repo)
        run("git", "commit", "-qm", "document", cwd=self.repo)
        self.head = run("git", "rev-parse", "HEAD", cwd=self.repo).stdout.strip()
        result = self.push()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.log.exists())

    def test_later_document_commit_does_not_hide_source_delta(self):
        run("git", "update-ref", "refs/remotes/origin/main", self.head, cwd=self.repo)
        (self.repo / "file").write_text("source change\n")
        run("git", "add", "file", cwd=self.repo)
        run("git", "commit", "-qm", "source", cwd=self.repo)
        (self.repo / "enablement/refactor.md").write_text("plan\n")
        run("git", "add", "enablement/refactor.md", cwd=self.repo)
        run("git", "commit", "-qm", "document", cwd=self.repo)
        self.head = run("git", "rev-parse", "HEAD", cwd=self.repo).stdout.strip()
        result = self.push()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("cargo build", self.log.read_text())

    def test_engine_only_push_does_not_build_root(self):
        run("git", "update-ref", "refs/remotes/origin/main", self.head, cwd=self.repo)
        tests = self.repo / "capture" / "engine" / "tests"
        tests.mkdir(parents=True)
        (tests / "process.rs").write_text("// Engine-only change\n")
        run("git", "add", ".", cwd=self.repo)
        run("git", "commit", "-qm", "engine test", cwd=self.repo)
        self.head = run("git", "rev-parse", "HEAD", cwd=self.repo).stdout.strip()
        result = self.push()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_text().splitlines(), [
            "-c utility cargo build --manifest-path capture/engine/Cargo.toml -j 3 --offline",
            "-c utility cargo test --manifest-path capture/engine/Cargo.toml -j 3 --offline -- --test-threads=2",
        ])

    def test_browser_only_push_runs_existing_package_checks_not_rust(self):
        run("git", "update-ref", "refs/remotes/origin/main", self.head, cwd=self.repo)
        browser = self.repo / "connect/web"
        browser.mkdir(parents=True, exist_ok=True)
        (browser / "package.json").write_text('{}\n')
        run("git", "add", ".", cwd=self.repo)
        run("git", "commit", "-qm", "independent browser", cwd=self.repo)
        self.head = run("git", "rev-parse", "HEAD", cwd=self.repo).stdout.strip()
        for tool in ("bun", "npm"):
            stub = self.bin / tool
            stub.write_text(f'#!/bin/sh\ncase "$*" in *enablement/ci/*) exit 0;; esac\nprintf "%s\\n" "{tool} $*" >> "$COMMAND_LOG"\nexit "${{MOCK_STATUS:-0}}"\n')
            stub.chmod(0o755)
        result = self.push()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_text().splitlines(), [
            "bun install --frozen-lockfile", "npm test",
        ])
        result = self.push(env=dict(self.env, MOCK_STATUS="1"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("verification failed", result.stderr)

    def test_ci_online_mode_uses_locked_and_propagates_failure(self):
        args = ("python3", "enablement/verification.py", "--base", "unknown", "--run", "--online")
        result = run(*args, cwd=self.repo, env=self.env)
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = self.log.read_text().splitlines()
        self.assertEqual(len(commands), 4)
        self.assertTrue(all("--locked" in line and "--offline" not in line for line in commands))
        self.log.unlink()
        result = run(*args, cwd=self.repo, env=dict(self.env, MOCK_STATUS="1"))
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len(self.log.read_text().splitlines()), 1)

    def test_tree_changed_during_verification_rejects_push(self):
        taskpolicy = self.bin / "taskpolicy"
        taskpolicy.write_text("#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$COMMAND_LOG\"\n"
                              "printf 'later\\n' > file\n")
        result = self.push()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("changed during verification", result.stderr)

    def test_dirty_tree_rejects_before_verification(self):
        (self.repo / "file").write_text("changed\n")
        result = self.push()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.log.exists())

    def test_hidden_index_flags_reject_before_verification(self):
        for flag in ("--assume-unchanged", "--skip-worktree"):
            with self.subTest(flag=flag):
                (self.repo / "file").write_text("different bytes\n")
                run("git", "update-index", flag, "file", cwd=self.repo)
                result = self.push()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("hidden index flags", result.stderr)
                self.assertFalse(self.log.exists())
                run("git", "update-index", "--no-assume-unchanged", "file", cwd=self.repo)
                run("git", "update-index", "--no-skip-worktree", "file", cwd=self.repo)
                (self.repo / "file").write_text("original\n")

    def test_other_revision_rejects_before_verification(self):
        run("git", "commit", "--allow-empty", "-qm", "newer", cwd=self.repo)
        result = self.push()
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.log.exists())

    def test_multiple_refs_cannot_hide_an_old_revision(self):
        newer = run("git", "commit", "--allow-empty", "-qm", "newer", cwd=self.repo)
        self.assertEqual(newer.returncode, 0)
        tip = run("git", "rev-parse", "HEAD", cwd=self.repo).stdout.strip()
        lines = (f"refs/heads/current {tip} refs/heads/current {'0' * 40}\n"
                 f"refs/heads/old {self.head} refs/heads/old {'0' * 40}\n")
        result = run("sh", str(HOOK), "origin", "example.invalid:repo", cwd=self.repo,
                     env=self.env, input=lines)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.log.exists())

    def test_ref_deletion_does_not_need_build(self):
        result = self.push(oid="0" * 40, local="(delete)")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.log.exists())

    def test_git_push_runs_installed_hook_and_blocks_failed_build(self):
        hooks = Path(self.tmp.name) / "hooks"
        hooks.mkdir()
        shutil.copy2(HOOK, hooks / "pre-push")
        (hooks / "pre-push").chmod(0o755)
        remote = Path(self.tmp.name) / "remote.git"
        run("git", "init", "--bare", "-q", str(remote), cwd=self.repo)
        run("git", "config", "core.hooksPath", str(hooks), cwd=self.repo)
        run("git", "remote", "add", "origin", str(remote), cwd=self.repo)

        failed = run("git", "push", "origin", "HEAD:refs/heads/main", cwd=self.repo,
                     env=dict(self.env, MOCK_STATUS="1"))
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("verification failed", failed.stderr)
        self.assertNotEqual(run("git", "show-ref", "--verify", "refs/heads/main",
                                cwd=remote).returncode, 0)

        passed = run("git", "push", "origin", "HEAD:refs/heads/main", cwd=self.repo,
                     env=self.env)
        self.assertEqual(passed.returncode, 0, passed.stderr)
        self.assertEqual(run("git", "rev-parse", "refs/heads/main", cwd=remote).stdout.strip(),
                         self.head)

        (self.repo / "file").write_text("hidden change\n")
        run("git", "update-index", "--assume-unchanged", "file", cwd=self.repo)
        hidden = run("git", "push", "origin", "HEAD:refs/heads/hidden", cwd=self.repo,
                     env=self.env)
        self.assertNotEqual(hidden.returncode, 0)
        self.assertIn("hidden index flags", hidden.stderr)
        self.assertNotEqual(run("git", "show-ref", "--verify", "refs/heads/hidden",
                                cwd=remote).returncode, 0)


if __name__ == "__main__":
    unittest.main()
