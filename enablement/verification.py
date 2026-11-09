#!/usr/bin/env python3
"""Conservative changed-tree verification shared by pre-push and Actions.

Repository CI runs contract checks. Pre-push owner selection is TypeScript in
local-tests/run.ts. Ready PRs and main run full platform CI; publication stays manual.
Explicit local --run retains conservative changed-tree checks.
Both sides of renames/deletions are included. Missing history broadens checks.
"""
import argparse
import json
import os
import re
import stat
import sys
from pathlib import Path
import subprocess
import time
from datetime import datetime, timezone


ROOT = Path(__file__).resolve().parents[1]
DOCS = {"AGENTS.md", "README.md", "enablement/refactor.md", "CONTEXT.md"}

# Exact reviewed extraction pairs, not an arbitrary Rust/mechanical classifier.
# f1b46d2a donors -> #221 1ea0d146 / #225 28d83f3d preserved bodies,
# tests and forwarding visibility. Only these files may differ; shared/native/
# build inputs still broaden. A newer donor or any body/test edit needs review.
# Git blob identities also bind whitespace, imports, cfgs and test-module wiring.
MOVES = {
    "config": {
        "blobs": {
            "src/handoff.rs": ("da2980adcf8e50b35019b043059d26c1697c8dd7", "7dcdf6b051b10d45d0d2e1afbfac5a5dc12ae393"),
            "src/handoff/config_store.rs": (None, "4d960d819900123d099aa6ad4d2a7827e3ffbfdc"),
            "src/handoff/test_modules.rs": (None, "9ac7c770ecb3aeb2d86892b5656330cf5292c405"),
            # Retained Mac fixture repair: canonicalize the owned test root,
            # not a product behavior change. Included in the pinned #221 slice.
            "src/hosted_logging_tests.rs": ("1d37007b970ca92183115e70a880c311aeaf7ea6", "d4f03bd3c4c1b220897d9d804058281f3f338ee1"),
        },
        "tests": ["handoff::", "hosted::", "settings::"],
    },
    # d59796c3 -> e86f515e: same three extraction blobs as config above.
    # Current main already contains the logging fixture repair; do not restore
    # its historical blob or require a fourth changed path for this pair.
    "config-current-main": {
        "blobs": {
            "src/handoff.rs": ("da2980adcf8e50b35019b043059d26c1697c8dd7", "7dcdf6b051b10d45d0d2e1afbfac5a5dc12ae393"),
            "src/handoff/config_store.rs": (None, "4d960d819900123d099aa6ad4d2a7827e3ffbfdc"),
            "src/handoff/test_modules.rs": (None, "9ac7c770ecb3aeb2d86892b5656330cf5292c405"),
        },
        "tests": ["handoff::", "hosted::", "settings::"],
    },
    "recorder": {
        "blobs": {
            "src/capture.rs": ("6d26953bf949d238ace1bf292801e1bc218875a9", "1f607ee1b7e20fb36f0801c18551fa58570165f2"),
            "capture/engine/runtime/recording/recorder.rs": (None, "a38a3c3c67ce4d01123a2578cdad14a2a7dddb8d"),
            "capture/engine/runtime/recording/recorder/tests.rs": (None, "7868cef5ac5073562b5bbefed2392606fdc555dc"),
        },
        "tests": ["agent_capture::", "capture::"],
    },
    # 84ef8f56: exact production block + all ten artifact test bodies
    # reconstructed against f1b46d2a; only imports and root forwarding differ.
    "processing": {
        "blobs": {
            "main.rs": ("a0ae40bc997f6e086ffeb27a44fe66cb15dddd02", "e3468e20f7f12cffd3c070dc2d8a19f281a2a230"),
            "src/bundle_processing.rs": (None, "88240b2b938639cfbdd190ad1209e5f306ac3f33"),
        },
        "tests": ["bundle_processing::", "capture::", "recover::", "sketch::"],
    },
    # d59796c3 -> f135448c: inverse of the reviewed extraction reconstructs
    # current main exactly, including shares CLI cases. The complete extracted
    # production/test module is unchanged from 84ef8f56.
    "processing-current-main": {
        "blobs": {
            "main.rs": ("b54efd65ae545d971c964b51b77fff7b2d4a3711", "7e8656f9e257d5fae635a47bdc7171c08604fca4"),
            "src/bundle_processing.rs": (None, "88240b2b938639cfbdd190ad1209e5f306ac3f33"),
        },
        "tests": ["bundle_processing::", "capture::", "recover::", "sketch::"],
    },
}

# Additive independent browser package from #200, inventory checked at c5bb1b78. Root
# build.rs/Cargo/native inputs do not consume this directory; its build reads
# the five approved public/site/public brand assets. Changing those/shared inputs is
# not this route. SharedLinks.tsx closure rechecked at 31281ba5; its existing
# standalone verifier covers shares routes. Unknown new owners remain broad.
BROWSER_FILES = {"connect/web/" + path for path in (
    ".gitignore", "AGENTS.md", "README.md", "VERIFICATION.md", "bun.lock",
    "index.html", "package.json", "tsconfig.json", "vite.config.ts",
    "scripts/assets.mjs", "scripts/prerender.mjs", "scripts/test-browser.mjs",
    "src/Hosted.tsx", "src/SharedLinks.tsx", "src/api.ts", "src/base.css", "src/entry-server.tsx",
    "src/hosted.css", "src/main.tsx", "test/artifact.test.mjs", "test/browser.mjs",
    "test/closure.test.mjs", "test/routes.test.mjs",
)}


def select(paths, history_known=True):
    paths = sorted(set(paths))
    # Unknown/shared/build/security/lifecycle input is broad by default. Do not
    # infer safety from a commit message, extension, or a newly created owner.
    broad = not history_known or not paths
    engine = False
    browser = False
    reasons = []
    for path in paths:
        if path in DOCS or (path.startswith("docs/") and path.endswith(".md")):
            continue
        if path.startswith("capture/engine/tests/") or path == "capture/engine/README.md":
            engine = True
        elif path in BROWSER_FILES:
            browser = True
        else:
            broad = True
            reasons.append(path)
    return {"root": broad, "engine": broad or engine, "browser": browser,
            "focused": [], "paths": paths,
            "reason": "unknown history or empty delta" if not history_known or not paths
            else "broad inputs: " + ", ".join(reasons) if broad else "document/package-test-only delta"}


def changed_paths(base, head):
    # --no-renames reports the old and new path, including cross-owner moves.
    result = subprocess.run(["git", "diff", "--no-renames", "--name-only", "-z",
                             base, head, "--"], cwd=ROOT, capture_output=True, check=True)
    return result.stdout.decode("utf-8").rstrip("\0").split("\0") if result.stdout else []


def plan(base, head):
    try:
        selection = select(changed_paths(base, head))
        remaining = set(selection["paths"])
        focused = []
        for recipe in MOVES.values():
            blobs = recipe["blobs"]
            if set(blobs) <= remaining and all(
                tree_entry(base, path) == (("100644", old) if old else None)
                and tree_entry(head, path) == ("100644", new)
                for path, (old, new) in blobs.items()
            ):
                remaining.difference_update(blobs)
                focused.extend(recipe["tests"])
        rest = select(remaining) if remaining else None
        if focused and (rest is None or not rest["root"]):
            selection.update(root=True, engine=bool(rest and rest["engine"]),
                             focused=sorted(set(focused)), reason="exact reviewed source-preserving extraction")
        return selection
    except (subprocess.CalledProcessError, UnicodeError, ValueError):
        return select([], history_known=False)


def tree_entry(revision, path):
    result = subprocess.run(["git", "ls-tree", "-z", revision, "--", path],
                            cwd=ROOT, capture_output=True, check=True)
    if not result.stdout:
        return None
    metadata, name = result.stdout.rstrip(b"\0").split(b"\t")
    mode, kind, oid = metadata.decode("ascii").split()
    return (mode, oid) if kind == "blob" else (mode, kind)


def check_temp_environment():
    # Rust uses TMPDIR on Unix, falling back to /tmp. Do not use Python's
    # tempfile fallback search: it can silently discard an invalid caller value.
    raw = os.environ.get("TMPDIR", "/tmp")
    root = Path(raw)
    remedy = "Set TMPDIR explicitly to a short, owned mode-0700 directory under trusted ancestors; no paths were changed."
    def reject(reason):
        raise RuntimeError(f"verification TMPDIR preflight: {reason}. {remedy}")
    if not raw or not root.is_absolute() or ".." in root.parts:
        reject("expected an absolute path without parent traversal")
    # Open each component relative to the checked parent; never follow symlinks.
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY)
    try:
        for index, part in enumerate(("", *root.parts[1:])):
            if part:
                try:
                    child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
                except OSError as error:
                    reject(f"component {index} is missing, inaccessible, or a symlink ({error.strerror})")
                os.close(fd)
                fd = child
            info = os.fstat(fd)
            sticky_root = info.st_uid == 0 and info.st_mode & stat.S_ISVTX
            if info.st_uid not in (0, os.geteuid()) or (info.st_mode & 0o022 and not sticky_root):
                reject(f"component {index} has untrusted ownership or writable permissions")
        if info.st_uid != os.geteuid() or stat.S_IMODE(info.st_mode) != 0o700:
            reject("temporary root must be owned by this user with mode 0700")
    finally:
        os.close(fd)
    # agent_capture::running_owner fixture: apparent-owner- + 32 hex ID,
    # then capture.sock. Include separator and terminating NUL headroom.
    suffix = ("/ac-" + "0" * 32 + "/s" if sys.platform == "darwin"
              else "/apparent-owner-" + "0" * 32 + "/capture.sock")
    limit = 103 if sys.platform == "darwin" else 107
    size = len(os.fsencode(raw.rstrip("/") + suffix))
    if size > limit:
        reject(f"socket fixture needs {size} pathname bytes; platform permits {limit}")


def run_pr_checks():
    subprocess.run(["python3", "shared/ui/scripts/check-theme.py"], cwd=ROOT, check=True)
    subprocess.run(["python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                    "-p", "test_*.py"], cwd=ROOT, check=True)
    subprocess.run(["node", "--experimental-strip-types", "enablement/local-tests/run.ts", "--contracts"],
                   cwd=ROOT, check=True)


def observed_compile(command, **kwargs):
    return observed_run(command, timing_label="rust-test-compilation", **kwargs)


def observed_run(command, timing_label=None, **kwargs):
    """Synthetic timing only; never changes command selection or acceptance."""
    if command[:3] == ["python3", "-m", "unittest"]:
        # Time the contract suite here, not its nested disposable verifiers.
        # Those fixtures must retain their ordinary command inventory and must
        # not append synthetic observations to the real job's timing artifact.
        child_env = dict(kwargs.get("env", os.environ))
        child_env.pop("APPARENT_TIMING_FILE", None)
        kwargs["env"] = child_env
    is_cargo = command[0] == "cargo" or command[:4] == ["taskpolicy", "-c", "utility", "cargo"]
    label = ("rust-build" if is_cargo and "build" in command else
             "rust-test-execution" if is_cargo and "test" in command else
             "contract-checks" if command[0] == "python3" else "browser-checks")
    label = timing_label or label
    if timing_label is None and is_cargo and "test" in command and os.environ.get("APPARENT_TIMING_FILE"):
        # Compile the identical test selection first; execution still uses Cargo
        # and retains every original filter/harness argument.
        compile_command = command[:command.index("--")] if "--" in command else list(command)
        compile_command += ["--no-run"]
        observed_compile(compile_command, **{key: value for key, value in kwargs.items()
                                          if key not in ("stdout", "text")})
    started = datetime.now(timezone.utc).isoformat()
    tick = time.monotonic()
    status = None
    try:
        result = subprocess.run(command, **kwargs)
        status = result.returncode
        return result
    except subprocess.CalledProcessError as error:
        status = error.returncode
        raise
    finally:
        output = os.environ.get("APPARENT_TIMING_FILE")
        if output:
            try:
                with open(output, "a") as stream:
                    stream.write(json.dumps({"schema": 1, "label": label,
                        "started": started, "elapsed_ms": round((time.monotonic() - tick) * 1000),
                        "code": status, "sha": os.environ.get("GITHUB_SHA")}) + "\n")
            except OSError:
                pass


def run_checks(selection, offline=True, execution="full", overlap_root_build=False):
    """Explicit CI split; ordinary local callers retain the complete inventory."""
    if execution not in ("full", "portable", "platform"):
        raise ValueError(f"Unknown execution scope: {execution}")
    portable = execution != "platform"
    native = execution != "portable"
    if native and (selection["root"] or selection["engine"]):
        check_temp_environment()
    if portable:
        observed_run(["python3", "shared/ui/scripts/check-theme.py"], cwd=ROOT, check=True)
        observed_run(["python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                        "-p", "test_verification.py"], cwd=ROOT, check=True)
    if native:
        observed_run(["python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                        "-p", "test_*push*.py"], cwd=ROOT, check=True)
        observed_run(["python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                        "-p", "test_temp_preflight.py"], cwd=ROOT, check=True)
    cargo = ["cargo"]
    if native and subprocess.run(["sh", "-c", "command -v taskpolicy"], capture_output=True).returncode == 0:
        cargo = ["taskpolicy", "-c", "utility", "cargo"]
    jobs = os.environ.get("T3_BUILD_JOBS", "3")
    dependency_mode = "--offline" if offline else "--locked"
    timings = ["--timings"] if os.environ.get("APPARENT_TIMING_FILE") else []
    if native and selection["root"]:
        # Only CI opts in. Cargo releases its compilation lock before its
        # build-finished event; two harness threads then overlap one app worker.
        overlap = overlap_root_build and jobs.isdigit() and int(jobs) >= 3 and not selection.get("focused")
        if overlap:
            commands = {
                "test": cargo + ["test", "-j", jobs, dependency_mode, *timings,
                                 "--message-format=json-render-diagnostics", "--", "--test-threads=2"],
                "build": cargo + ["build", "-j", "1", dependency_mode] + timings,
            }
            subprocess.run(["node", "--experimental-strip-types", str(ROOT / "enablement/ci/root.ts"),
                            json.dumps(commands)], cwd=ROOT, check=True)
        else:
            observed_run(cargo + ["build", "-j", jobs, dependency_mode] + timings, cwd=ROOT, check=True)
            if selection.get("focused"):
                for test_filter in selection["focused"]:
                    result = observed_run(cargo + ["test", "-j", jobs, dependency_mode, *timings, test_filter,
                                                    "--", "--test-threads=2"], cwd=ROOT,
                                            stdout=subprocess.PIPE, text=True, check=True)
                    print(result.stdout, flush=True)
                    if not any(int(n) > 0 for n in re.findall(r"test result: ok\. (\d+) passed", result.stdout)):
                        raise RuntimeError(f"Focused filter selected no passing tests: {test_filter}")
            else:
                observed_run(cargo + ["test", "-j", jobs, dependency_mode, *timings, "--", "--test-threads=2"], cwd=ROOT, check=True)
    if native and selection["root"] and not selection.get("focused"):
        # Rust cfg(test) suppresses runtime experiment defaults. Check the rebuilt
        # production CLI too, using isolated config with no upload permission.
        candidate = (ROOT / os.environ.get("CARGO_TARGET_DIR", "target") / "debug/apparent").resolve()
        observed_run(["node", "--experimental-strip-types", "--test",
                      "connect/client/tests/encryption-default.test.ts"], cwd=ROOT,
                     env=dict(os.environ, APPARENT_BIN=str(candidate)), check=True)
    if native and selection["engine"]:
        for command in ("build", "test"):
            args = cargo + [command, "--manifest-path", "capture/engine/Cargo.toml", "-j", jobs, dependency_mode] + timings
            if command == "test":
                args += ["--", "--test-threads=2"]
            observed_run(args, cwd=ROOT, check=True)
    if portable and selection.get("browser"):
        # Use the existing package fixtures, including the separate loopback
        # build. Do not substitute a production artifact with synthetic config.
        browser = ROOT / "connect/web"
        for command in (["bun", "install", "--frozen-lockfile"], ["bun", "run", "build"],
                        ["node", "--test", "test/artifact.test.mjs", "test/closure.test.mjs", "test/routes.test.mjs"],
                        ["node", "scripts/test-browser.mjs"]):
            observed_run(command, cwd=browser, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--preflight", action="store_true", help="Check temporary environment only; no builds or mutations")
    parser.add_argument("--head", default="HEAD")
    parser.add_argument("--run", action="store_true")
    parser.add_argument("--pr", action="store_true", help="Hook/CI: cheap PR contract checks")
    parser.add_argument("--online", action="store_true", help="CI: use locked graph instead of offline cache")
    parser.add_argument("--execution", choices=("full", "portable", "platform"), default="full",
                        help="Explicit CI routing; local default runs portable and platform checks")
    parser.add_argument("--overlap-root-build", action="store_true",
                        help="CI: overlap two root test threads with one required app-build worker")
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()
    if args.preflight:
        check_temp_environment()
        print("Temporary environment prerequisites passed; not native proof.")
        return
    selection = plan(args.base, args.head)
    print(json.dumps(selection, sort_keys=True), flush=True)
    if args.github_output:
        with args.github_output.open("a") as output:
            for key in ("root", "engine", "browser"):
                output.write(f"{key}={str(selection[key]).lower()}\n")
    if args.pr:
        run_pr_checks()
    if args.run:
        run_checks(selection, offline=not args.online, execution=args.execution,
                   overlap_root_build=args.overlap_root_build)


if __name__ == "__main__":
    main()
