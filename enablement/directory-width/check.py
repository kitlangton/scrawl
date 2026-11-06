#!/usr/bin/env python3
"""Reject staged changes that widen an affected first-party code directory."""
from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys

LIMIT = 10
# Dot-prefixed immediate root directories are configuration/tooling areas and
# are excluded from the ordinary owner-directory count. Their contents remain
# eligible and checked at their own levels.
# GitHub Actions requires each workflow entrypoint to be an immediate file.
# This exact inventory is the sole file-count exception; unknown entries fail.
WORKFLOW_DIRECTORY = b".github/workflows"
WORKFLOW_FILES = {
    b"connect-delivery.yml", b"linux-updater-harness.yml", b"native-proof-mac.yml",
    b"release-linux.yml", b"release-mac-preview.yml", b"release-windows.yml",
    b"verification-mac.yml",
    b"verification-platform.yml", b"verification.yml", b"web-delivery.yml",
    b"stream-agent.yml", b"stream-app-shell.yml", b"stream-connect-client.yml", b"stream-connect-browser.yml",
    b"stream-engine.yml",
    b"stream-onboarding.yml", b"stream-platform-config.yml", b"stream-platform-resources.yml", b"stream-public.yml",
    b"stream-record.yml",
}
WORKFLOW_DIRECTORIES: set[bytes] = set()
# Eligibility is by tracked first-party tree scope, not filename suffix. Every
# non-excluded blob is repo-owned content for width purposes (including files
# with extensionless or uncommon source names). Keep generated/dependency
# exclusions explicit and narrow above.
# Deliberately narrow generated/dependency exclusions. Do not exempt generic
# source roots (notably src/shared), library crates, or arbitrary "dist" dirs.
EXCLUDED_COMPONENTS = {
    b".git", b"target", b"node_modules", b"vendor", b"third_party",
    b"third-party", b"__pycache__",
}
EXCLUDED_PREFIXES = (
    b"public/site/video-demo/output/",   # generated demo render output
    b"connect/web/dist/",                # generated browser package output
    b"connect/web/dist-site-check/",     # generated verification output
)


def git(root: Path, *args: str) -> bytes:
    return subprocess.run(["git", *args], cwd=root, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, check=True).stdout


def decode_path(path: bytes) -> str:
    return path.decode("utf-8", "backslashreplace")


def excluded(path: bytes) -> bool:
    if path.startswith(EXCLUDED_PREFIXES):
        return True
    return any(part in EXCLUDED_COMPONENTS for part in path.split(b"/"))


def is_first_party_file(path: bytes) -> bool:
    return not excluded(path)


def tree_entries(root: Path, treeish: str) -> dict[bytes, tuple[bytes, bytes]]:
    raw = git(root, "ls-tree", "-rz", "--full-tree", treeish)
    entries: dict[bytes, tuple[bytes, bytes]] = {}
    for record in raw.split(b"\0"):
        if not record:
            continue
        metadata, path = record.split(b"\t", 1)
        mode, kind, _oid = metadata.split(b" ", 2)
        if excluded(path):
            continue
        entries[path] = (mode, kind)
    return entries


def directories(entries: dict[bytes, tuple[bytes, bytes]]) -> set[bytes]:
    return {path[:i] for path in entries for i, byte in enumerate(path) if byte == 47}


def code_directories(entries: dict[bytes, tuple[bytes, bytes]]) -> set[bytes]:
    result: set[bytes] = set()
    for path, (mode, kind) in entries.items():
        if kind != b"blob" or mode == b"160000" or not is_first_party_file(path):
            continue
        parent = path.rpartition(b"/")[0]
        result.add(parent)
        while parent:
            parent = parent.rpartition(b"/")[0]
            result.add(parent)
    return result


def children(entries: dict[bytes, tuple[bytes, bytes]]) -> dict[bytes, dict[bytes, str]]:
    result: dict[bytes, dict[bytes, str]] = {}
    for path, (mode, _kind) in entries.items():
        parts = path.split(b"/")
        parent = b""
        for part in parts[:-1]:
            result.setdefault(parent, {})[part] = "directory"
            parent = part if not parent else parent + b"/" + part
            result.setdefault(parent, {})
        final_kind = "directory" if mode == b"160000" else "file"
        result.setdefault(parent, {})[parts[-1]] = final_kind
    return result


def changed_paths(root: Path, head_exists: bool) -> list[bytes]:
    args = ["diff", "--cached", "--no-renames", "--name-only", "-z"]
    if head_exists:
        args.append("HEAD")
    else:
        args.append("--root")
    args.append("--")
    raw = git(root, *args)
    return [path for path in raw.split(b"\0") if path]


def inspect(root: Path) -> list[tuple[bytes, int, int]]:
    head_result = subprocess.run(["git", "rev-parse", "--verify", "HEAD"], cwd=root,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    head_exists = head_result.returncode == 0
    if head_result.returncode and b"Needed a single revision" not in head_result.stderr:
        raise RuntimeError("cannot determine HEAD: " + head_result.stderr.decode("utf-8", "replace").strip())
    staged = changed_paths(root, head_exists)
    if not staged:
        return []

    # write-tree reads the index candidate without changing it. HEAD is read as
    # immutable tree data; no worktree path or file bytes participate.
    candidate_tree = git(root, "write-tree").decode("ascii").strip()
    after = tree_entries(root, candidate_tree)
    before = tree_entries(root, "HEAD^{tree}") if head_exists else {}
    after_children, before_children = children(after), children(before)
    affected: set[bytes] = set()
    for path in staged:
        if excluded(path):
            continue
        parent = path.rpartition(b"/")[0]
        if parent:
            affected.add(parent)
        elif is_first_party_file(path):
            # Editing an immediate first-party root file is a root-owned change.
            affected.add(b"")
    # Include a directory when its immediate entries changed, including root.
    # Ordinary nested file edits do not affect ancestors whose entries stay put.
    all_dirs = set(after_children) | set(before_children)
    for directory in all_dirs:
        if before_children.get(directory, {}) != after_children.get(directory, {}):
            affected.add(directory)
    code_dirs = code_directories(after) | code_directories(before)

    violations = []
    for directory in sorted(affected & code_dirs):
        direct = after_children.get(directory, {})
        files = sum(kind == "file" for kind in direct.values())
        dirs = sum(kind == "directory" for kind in direct.values())
        if directory == b"":
            normal_dirs = sum(
                kind == "directory" and not name.startswith(b".")
                for name, kind in direct.items()
            )
            width_exceeded = files > LIMIT or normal_dirs > LIMIT
        elif directory == WORKFLOW_DIRECTORY:
            normal_dirs = dirs
            width_exceeded = dirs > LIMIT
        else:
            normal_dirs = dirs
            width_exceeded = files > LIMIT or dirs > LIMIT
        unknown_workflow_entries = (
            directory == WORKFLOW_DIRECTORY
            and bool(set(direct) - WORKFLOW_FILES - WORKFLOW_DIRECTORIES)
        )
        workflow_file_overflow = directory == WORKFLOW_DIRECTORY and files > len(WORKFLOW_FILES)
        if width_exceeded or unknown_workflow_entries or workflow_file_overflow:
            violations.append((directory, files, dirs))
    return violations


def main() -> int:
    root = Path(__file__).resolve().parents[2]
    try:
        # Confirm this is the repository Git resolves from the hook's location.
        actual_root = Path(os.fsdecode(git(root, "rev-parse", "--show-toplevel").rstrip(b"\n"))).resolve()
        if actual_root != root.resolve():
            raise RuntimeError(f"hook root mismatch: {actual_root}")
        violations = inspect(root)
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        detail = getattr(error, "stderr", b"")
        if isinstance(detail, bytes):
            detail = detail.decode("utf-8", "replace").strip()
        print(f"pre-commit: directory-width check failed closed: {error} {detail}".rstrip(), file=sys.stderr)
        return 1
    if violations:
        print("pre-commit: staged candidate exceeds the directory-width limit:", file=sys.stderr)
        for path, files, dirs in violations:
            if path == b"":
                print(f"  <root>: {files} files, {dirs} subdirectories (limit 10 files, 10 non-hidden directories)", file=sys.stderr)
            elif path == WORKFLOW_DIRECTORY:
                print(f"  {decode_path(path)}: {files} files, {dirs} subdirectories (limit 11 listed files, 10 subdirectories)", file=sys.stderr)
            else:
                print(f"  {decode_path(path)}: {files} files, {dirs} subdirectories (limit 10 each)", file=sys.stderr)
        print("Split changes by responsibility and stage only the intended changes; this hook does not edit or stage files.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
