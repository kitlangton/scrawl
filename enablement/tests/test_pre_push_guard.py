"""Exercise the actual pre-push hook in disposable Git repositories."""

import os
from pathlib import Path
import shutil
import subprocess
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
        self.tmp = tempfile.TemporaryDirectory(prefix=".pf-", dir=Path.home())
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
        # File-length tests exercise pre-commit only, so this closure stays
        # finite and never recursively invokes the pre-push test suite.
        for name in (".githooks/pre-commit", "enablement/directory-width/check.py",
                     "enablement/file-length/check.ts", "enablement/file-length/check.test.ts"):
            destination = self.repo / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(HOOK.parents[1] / name, destination)
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

    def test_feature_push_runs_contracts_without_cargo(self):
        result = self.push(remote="refs/heads/feature")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.log.exists())

    def onboarding_delta(self, dependencies=True, delete=False, rename=False):
        frontend = self.repo / "capture/desktop/ui/frontend"
        entry = frontend / "src/onboarding/main.tsx"
        entry.parent.mkdir(parents=True, exist_ok=True)
        entry.write_text("// original entry\n")
        if dependencies:
            for name in ("typescript/bin/tsc", "vite/bin/vite.js", "@apparent/ui/package.json"):
                target = frontend / "node_modules" / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text("fixture\n")
        run("git", "add", ".", cwd=self.repo)
        run("git", "commit", "-qm", "entry baseline", cwd=self.repo)
        run("git", "update-ref", "refs/remotes/origin/main", "HEAD", cwd=self.repo)
        if rename:
            entry.rename(entry.with_name("renamed.tsx"))
        elif delete:
            entry.unlink()
        else:
            entry.write_text("// changed entry\n")
        run("git", "add", "-A", cwd=self.repo)
        run("git", "commit", "-qm", "entry delta", cwd=self.repo)
        self.head = run("git", "rev-parse", "HEAD", cwd=self.repo).stdout.strip()
        npm = self.bin / "npm"
        npm.write_text('#!/bin/sh\nprintf "%s:%s\\n" "$PWD" "$*" >> "$COMMAND_LOG"\nexit "${NPM_STATUS:-0}"\n')
        npm.chmod(0o755)
        return frontend

    def test_onboarding_feature_delta_runs_existing_command_in_frontend(self):
        frontend = self.onboarding_delta()
        result = self.push(remote="refs/heads/feature")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_text(), f"{frontend}:run check:onboarding\n")

    def test_onboarding_command_failure_rejects_feature_push(self):
        self.onboarding_delta()
        result = self.push(remote="refs/heads/feature", env=dict(self.env, NPM_STATUS="1"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("push rejected", result.stderr)

    def test_transitive_onboarding_inputs_run_existing_renderer_check(self):
        frontend = self.onboarding_delta()
        for path in (
                "capture/desktop/ui/frontend/src/onboarding/form.tsx",
                "capture/desktop/ui/frontend/src/shared/theme.css",
                "capture/desktop/ui/frontend/vite.config.ts",
                "capture/desktop/ui/frontend/package.json",
                "shared/ui/src/button.tsx"):
            target = self.repo / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text("// changed transitive input\n")
        run("git", "add", "-A", cwd=self.repo)
        run("git", "commit", "-qm", "transitive inputs", cwd=self.repo)
        self.head = run("git", "rev-parse", "HEAD", cwd=self.repo).stdout.strip()
        result = self.push(remote="refs/heads/feature")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_text(), f"{frontend}:run check:onboarding\n")

    def test_unrelated_public_owner_skips_onboarding_renderer_check(self):
        frontend = self.onboarding_delta()
        entry = frontend / "src/onboarding/main.tsx"
        entry.write_text("// original entry\n")
        target = self.repo / "public/site/src/features.json"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("{}\n")
        run("git", "add", "-A", cwd=self.repo)
        run("git", "commit", "-qm", "public owner input", cwd=self.repo)
        self.head = run("git", "rev-parse", "HEAD", cwd=self.repo).stdout.strip()
        result = self.push(remote="refs/heads/feature")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.log.exists())

    def test_onboarding_missing_dependencies_never_install(self):
        self.onboarding_delta(dependencies=False)
        result = self.push(remote="refs/heads/feature")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("run npm ci in capture/desktop/ui/frontend", result.stderr)
        self.assertFalse(self.log.exists())

    def test_onboarding_deleted_or_renamed_entry_requires_contract_update(self):
        for rename in (False, True):
            with self.subTest(rename=rename):
                self.onboarding_delta(delete=not rename, rename=rename)
                result = self.push(remote="refs/heads/feature")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("deleted or renamed", result.stderr)

    def test_clean_head_requires_build_and_tests(self):
        result = self.push()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_text().splitlines(), [
            "-c utility cargo build --manifest-path capture/engine/Cargo.toml -j 3 --offline",
            "-c utility cargo test --manifest-path capture/engine/Cargo.toml -j 3 --offline -- --test-threads=2",
            "-c utility cargo test -j 3 --offline -- --test-threads=2",
            "-c utility cargo build -j 3 --offline",
        ])

    def test_failed_verification_rejects_push(self):
        result = self.push(env=dict(self.env, MOCK_STATUS="1"))
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("failed", result.stderr)

    def test_explicit_build_allocation_reaches_all_cargo_commands(self):
        result = self.push(env=dict(self.env, T3_BUILD_JOBS="7"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_text().splitlines(), [
            "-c utility cargo build --manifest-path capture/engine/Cargo.toml -j 7 --offline",
            "-c utility cargo test --manifest-path capture/engine/Cargo.toml -j 7 --offline -- --test-threads=2",
            "-c utility cargo test -j 7 --offline -- --test-threads=2",
            "-c utility cargo build -j 7 --offline",
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
        browser.mkdir(parents=True)
        (browser / "package.json").write_text('{}\n')
        run("git", "add", ".", cwd=self.repo)
        run("git", "commit", "-qm", "independent browser", cwd=self.repo)
        self.head = run("git", "rev-parse", "HEAD", cwd=self.repo).stdout.strip()
        for tool in ("bun", "node"):
            stub = self.bin / tool
            stub.write_text(f'#!/bin/sh\nprintf "%s\\n" "{tool} $*" >> "$COMMAND_LOG"\nexit "${{MOCK_STATUS:-0}}"\n')
            stub.chmod(0o755)
        result = self.push()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.log.read_text().splitlines(), [
            "bun install --frozen-lockfile", "bun run build",
            "node --test test/artifact.test.mjs test/closure.test.mjs test/routes.test.mjs",
            "node scripts/test-browser.mjs",
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
