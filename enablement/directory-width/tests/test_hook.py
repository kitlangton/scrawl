from __future__ import annotations

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
HOOK = ROOT / ".githooks/pre-commit"
CHECKER = ROOT / "enablement/directory-width/check.py"


class DirectoryWidthHookTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp = tempfile.TemporaryDirectory(prefix="width-hook-")
        self.repo = Path(self.temp.name) / "repo"
        self.repo.mkdir()
        self.env = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        self.env.update({"HOME": str(Path(self.temp.name) / "home"), "PYTHONDONTWRITEBYTECODE": "1"})
        Path(self.env["HOME"]).mkdir()
        self.git("init", "-q")
        self.git("config", "user.name", "Directory Width Test")
        self.git("config", "user.email", "width@example.invalid")
        (self.repo / ".githooks").mkdir()
        shutil.copy2(HOOK, self.repo / ".githooks/pre-commit")
        (self.repo / ".githooks/pre-commit").chmod(0o755)
        target = self.repo / "enablement/directory-width"
        target.mkdir(parents=True)
        shutil.copy2(CHECKER, target / "check.py")
        self.git("config", "core.hooksPath", ".githooks")

    def tearDown(self) -> None:
        self.temp.cleanup()

    def git(self, *args: str, check: bool = True) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run(["git", *args], cwd=self.repo, env=self.env,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=check)

    def put(self, path: str, content: str = "x\n") -> None:
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)

    def stage(self, *paths: str) -> None:
        self.git("add", "--", *paths)

    def commit(self, *paths: str) -> None:
        if paths:
            self.stage(*paths)
        self.git("-c", "core.hooksPath=/dev/null", "commit", "-qm", "fixture")

    def run_hook(self) -> subprocess.CompletedProcess[bytes]:
        return subprocess.run([str(self.repo / ".githooks/pre-commit")], cwd=self.repo, env=self.env,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def seed_code_files(self, count: int, directory: str = "src/shared") -> list[str]:
        paths = [f"{directory}/item-{i:02d}.rs" for i in range(count)]
        for path in paths:
            self.put(path)
        return paths

    def test_ten_immediate_files_allowed_and_eleven_blocked(self) -> None:
        paths = self.seed_code_files(10)
        self.stage(*paths)
        allowed = self.run_hook()
        self.assertEqual(allowed.returncode, 0, allowed.stderr.decode())

        self.put("src/shared/item-10.rs")
        self.stage("src/shared/item-10.rs")
        blocked = self.run_hook()
        self.assertEqual(blocked.returncode, 1)
        self.assertIn(b"src/shared: 11 files, 0 subdirectories (limit 10 each)", blocked.stderr)
        self.assertIn(b"Split changes by responsibility and stage", blocked.stderr)

    def test_eleven_immediate_subdirectories_block(self) -> None:
        for i in range(11):
            self.put(f"src/shared/part-{i:02d}/lib.rs")
        self.stage("src/shared")
        result = self.run_hook()
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"src/shared: 0 files, 11 subdirectories (limit 10 each)", result.stderr)

    def test_unrelated_staged_change_does_not_enforce_existing_overwidth(self) -> None:
        files = self.seed_code_files(11)
        self.commit(*files)
        self.put("README.md", "unrelated text\n")
        self.stage("README.md")
        result = self.run_hook()
        self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_nested_file_edit_does_not_enforce_existing_root_width(self) -> None:
        root_files = [f"root-{i:02d}.txt" for i in range(11)]
        for path in root_files:
            self.put(path)
        self.put("build.rs")
        self.put("src/shared/base.rs")
        self.commit(*root_files, "build.rs", "src/shared/base.rs")

        self.put("src/shared/edited.rs")
        self.stage("src/shared/edited.rs")
        result = self.run_hook()
        self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_root_code_file_edit_enforces_existing_root_width(self) -> None:
        root_files = [f"root-{i:02d}.txt" for i in range(11)]
        for path in root_files:
            self.put(path)
        self.put("build.rs", "fn main() {}\n")
        self.commit(*root_files, "build.rs")

        self.put("build.rs", 'fn main() { println!("changed"); }\n')
        self.stage("build.rs")
        result = self.run_hook()
        self.assertEqual(result.returncode, 1)
        self.assertIn(b": 12 files, 0 subdirectories (limit 10 each)", result.stderr)

    def test_top_level_entry_change_enforces_root_width(self) -> None:
        for i in range(11):
            self.put(f"root-dir-{i:02d}/mod.rs")
        self.stage("root-dir-*")
        result = self.run_hook()
        self.assertEqual(result.returncode, 1)
        self.assertIn(b": 0 files, 11 subdirectories (limit 10 each)", result.stderr)

    def test_tracked_first_party_html_beneath_public_root_is_enforced(self) -> None:
        paths = [f"public/site/public/feature-demos/page-{i}.html" for i in range(11)]
        for path in paths:
            self.put(path)
        self.stage(*paths)
        result = self.run_hook()
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"public/site/public/feature-demos: 11 files", result.stderr)

    def test_generated_published_homepage_bundle_is_excluded(self) -> None:
        paths = [f"public/site/published-homepage/assets/generated-{i}.js" for i in range(11)]
        for path in paths:
            self.put(path)
        self.stage(*paths)
        result = self.run_hook()
        self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_unmerged_index_fails_closed(self) -> None:
        blobs = []
        for content in (b"base\n", b"conflict\n"):
            result = subprocess.run(["git", "hash-object", "-w", "--stdin"], cwd=self.repo,
                                    env=self.env, input=content, stdout=subprocess.PIPE,
                                    stderr=subprocess.PIPE, check=True)
            blobs.append(result.stdout.decode().strip())
        records = "100644 " + blobs[0] + " 1\tconflict.rs\n"
        records += "100644 " + blobs[1] + " 2\tconflict.rs\n"
        subprocess.run(["git", "update-index", "--index-info"], cwd=self.repo, env=self.env,
                       input=records.encode(), stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                       check=True)

        result = self.run_hook()
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"directory-width check failed closed", result.stderr)

    def test_hook_uses_index_not_worktree_in_either_direction(self) -> None:
        paths = self.seed_code_files(10)
        self.stage(*paths)
        self.put("src/shared/unstaged.rs")
        allowed = self.run_hook()
        self.assertEqual(allowed.returncode, 0, allowed.stderr.decode())

        self.put("src/shared/staged.rs")
        self.stage("src/shared/staged.rs")
        # Remove a different staged file only from the worktree; index still has 11.
        (self.repo / paths[0]).unlink()
        blocked = self.run_hook()
        self.assertEqual(blocked.returncode, 1)
        self.assertIn(b"src/shared: 11 files", blocked.stderr)

    def test_deletion_can_reduce_overwidth(self) -> None:
        paths = self.seed_code_files(11)
        self.commit(*paths)
        (self.repo / paths[0]).unlink()
        self.stage(paths[0])
        result = self.run_hook()
        self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_rename_can_reduce_overwidth(self) -> None:
        paths = self.seed_code_files(11)
        self.commit(*paths)
        self.git("mv", paths[0], "src/renamed.rs")
        result = self.run_hook()
        self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_new_first_party_directory_with_extensionless_files_is_checked(self) -> None:
        paths = [f"capture/engine/new-owner/file-{i:02d}" for i in range(11)]
        for path in paths:
            self.put(path)
        self.stage(*paths)
        result = self.run_hook()
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"capture/engine/new-owner: 11 files, 0 subdirectories (limit 10 each)", result.stderr)

    def test_initial_commit_candidate_is_checked(self) -> None:
        paths = self.seed_code_files(11)
        self.stage(*paths)
        result = self.run_hook()
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"src/shared: 11 files", result.stderr)

    def test_initial_commit_at_limit_is_allowed(self) -> None:
        paths = self.seed_code_files(10)
        self.stage(*paths)
        result = self.run_hook()
        self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_dependency_and_known_generated_roots_are_excluded(self) -> None:
        paths = [f"vendor/pkg-{i}/lib.rs" for i in range(11)]
        paths += [f"public/site/published-homepage/assets/generated-{i}.js" for i in range(11)]
        paths += [f"target/gen-{i}/mod.rs" for i in range(11)]
        for path in paths:
            self.put(path)
        self.stage(*paths)
        result = self.run_hook()
        self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_nested_first_party_library_directory_is_enforced(self) -> None:
        paths = [f"capture/engine/src/modules/unit-{i}.rs" for i in range(11)]
        for path in paths:
            self.put(path)
        self.stage(*paths)
        result = self.run_hook()
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"capture/engine/src/modules: 11 files", result.stderr)

    def test_spaces_newlines_and_symlinks_are_counted_without_following_them(self) -> None:
        for i in range(9):
            self.put(f"src/shared/file-{i}.rs")
        self.put("src/shared/space name.rs")
        outside = Path(self.temp.name) / "outside.rs"
        outside.write_text("not followed\n")
        os.symlink(outside, self.repo / "src/shared/newline\nname.rs")
        self.stage("src/shared")
        result = self.run_hook()
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"src/shared: 11 files", result.stderr)

    def test_submodule_gitlink_counts_as_directory_entry(self) -> None:
        nested = Path(self.temp.name) / "nested"
        nested.mkdir()
        subprocess.run(["git", "init", "-q", str(nested)], env=self.env, check=True)
        subprocess.run(["git", "-C", str(nested), "config", "user.name", "Nested"], env=self.env, check=True)
        subprocess.run(["git", "-C", str(nested), "config", "user.email", "nested@example.invalid"], env=self.env, check=True)
        (nested / "file").write_text("x")
        subprocess.run(["git", "-C", str(nested), "add", "file"], env=self.env, check=True)
        subprocess.run(["git", "-C", str(nested), "commit", "-qm", "nested"], env=self.env, check=True)
        oid = subprocess.run(["git", "-C", str(nested), "rev-parse", "HEAD"], env=self.env,
                             stdout=subprocess.PIPE, check=True).stdout.decode().strip()
        self.put("src/shared/base.rs")
        self.stage("src/shared/base.rs")
        self.git("update-index", "--add", "--cacheinfo", "160000," + oid + ",src/shared/module")
        result = self.run_hook()
        self.assertEqual(result.returncode, 0, result.stderr.decode())


if __name__ == "__main__":
    unittest.main()
