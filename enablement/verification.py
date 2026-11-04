#!/usr/bin/env python3
"""Conservative changed-tree verification shared by pre-push and Actions.

PR pushes run contract checks; main CI runs full platform verification before CD.
Explicit local --run retains conservative changed-tree checks.
Both sides of renames/deletions are included. Missing history broadens checks.
"""
import argparse
import json
import os
import re
import stat
import signal
import sys
from pathlib import Path
import subprocess
from contextlib import contextmanager


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
    for pattern in ("test_*push*.py", "test_verification.py", "test_release_gate.py",
                    "test_temp_preflight.py", "test_owner_lanes.py"):
        subprocess.run(["python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                        "-p", pattern], cwd=ROOT, check=True)


def run_contract_checks():
    subprocess.run(["python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                    "-p", "test_*push*.py"], cwd=ROOT, check=True)
    subprocess.run(["python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                    "-p", "test_verification.py"], cwd=ROOT, check=True)
    subprocess.run(["python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                    "-p", "test_temp_preflight.py"], cwd=ROOT, check=True)


@contextmanager
def overlapping_contracts():
    """Own one isolated contract process group until both verification paths finish."""
    process = None
    cancelled = None
    def interrupted(signum, _frame):
        nonlocal cancelled
        cancelled = 128 + signum
        if process is not None:
            raise SystemExit(cancelled)
    previous = {sig: signal.signal(sig, interrupted)
                for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT)}
    try:
        process = subprocess.Popen([sys.executable, str(Path(__file__).resolve()), '--contract-checks'],
                                   cwd=ROOT, start_new_session=True)
        if cancelled is not None:
            raise SystemExit(cancelled)
        yield
        status = process.wait()
        if status:
            raise subprocess.CalledProcessError(status, process.args)
    finally:
        if process is not None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def run_checks(selection, offline=True, parallel_contracts=False, cargo_timings=False):
    if selection["root"] or selection["engine"]:
        check_temp_environment()
    subprocess.run(["python3", "shared/ui/scripts/check-theme.py"], cwd=ROOT, check=True)
    if parallel_contracts:
        with overlapping_contracts():
            run_selected_checks(selection, offline, cargo_timings)
    else:
        run_contract_checks()
        run_selected_checks(selection, offline, cargo_timings)


def run_selected_checks(selection, offline=True, cargo_timings=False):
    cargo = ["cargo"]
    if subprocess.run(["sh", "-c", "command -v taskpolicy"], capture_output=True).returncode == 0:
        cargo = ["taskpolicy", "-c", "utility", "cargo"]
    jobs = os.environ.get("T3_BUILD_JOBS", "3")
    dependency_mode = "--offline" if offline else "--locked"
    diagnostics = ["--timings"] if cargo_timings else []
    if selection["root"]:
        if selection.get("focused"):
            for test_filter in selection["focused"]:
                result = subprocess.run(cargo + ["test", "-j", jobs, dependency_mode] + diagnostics + [test_filter,
                                                "--", "--test-threads=2"], cwd=ROOT,
                                        stdout=subprocess.PIPE, text=True, check=True)
                print(result.stdout, flush=True)
                if not any(int(n) > 0 for n in re.findall(r"test result: ok\. (\d+) passed", result.stdout)):
                    raise RuntimeError(f"Focused filter selected no passing tests: {test_filter}")
        else:
            subprocess.run(cargo + ["test", "-j", jobs, dependency_mode] + diagnostics + ["--", "--test-threads=2"], cwd=ROOT, check=True)
        # Keep the preflight test harness warm before building the distinct app
        # binary required by subsequent bridge, packaging and native checks.
        subprocess.run(cargo + ["build", "-j", jobs, dependency_mode] + diagnostics, cwd=ROOT, check=True)
    if selection["engine"]:
        for command in ("build", "test"):
            args = cargo + [command, "--manifest-path", "capture/engine/Cargo.toml", "-j", jobs, dependency_mode] + diagnostics
            if command == "test":
                args += ["--", "--test-threads=2"]
            subprocess.run(args, cwd=ROOT, check=True)
    if selection.get("browser"):
        # Use the existing package fixtures, including the separate loopback
        # build. Do not substitute a production artifact with synthetic config.
        browser = ROOT / "connect/web"
        for command in (["bun", "install", "--frozen-lockfile"], ["bun", "run", "build"],
                        ["node", "--test", "test/artifact.test.mjs", "test/closure.test.mjs", "test/routes.test.mjs"],
                        ["node", "scripts/test-browser.mjs"]):
            subprocess.run(command, cwd=browser, check=True)


def hook_budget(seconds, command):
    """Bound the whole hook, including descendants, on macOS and Linux."""
    process = None
    cancelled = None
    def interrupted(signum, _frame):
        nonlocal cancelled
        cancelled = 128 + signum
        # Popen may already have created the child before returning its handle.
        # Record cancellation until we can kill/reap that owned process group.
        if process is not None:
            raise SystemExit(cancelled)

    previous = {sig: signal.signal(sig, interrupted)
                for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT)}
    try:
        process = subprocess.Popen(command, start_new_session=True)
        if cancelled is not None:
            return cancelled
        return process.wait(timeout=seconds)
    except subprocess.TimeoutExpired:
        print(f"Verification exceeded {seconds:g}s. Optimize the tests: the agent "
              "whose run hit this limit owns the optimization and successful rerun. "
              "Do not raise or bypass the limit.", file=sys.stderr, flush=True)
        return 124
    finally:
        # A successful parent must not leave background verification running.
        if process is not None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", required=True)
    parser.add_argument("--preflight", action="store_true", help="Check temporary environment only; no builds or mutations")
    parser.add_argument("--head", default="HEAD")
    parser.add_argument("--receipt-dir", type=Path, help="Write observation-only receipts outside the checkout (Node 22 required)")
    parser.add_argument("--run", action="store_true")
    parser.add_argument("--suite", choices=("all", "rust", "browser"), default="all",
                        help="Execute a full platform suite; default runs all changed-tree selections")
    parser.add_argument("--pr", action="store_true", help="Hook/CI: cheap PR contract checks")
    parser.add_argument("--online", action="store_true", help="CI: use locked graph instead of offline cache")
    parser.add_argument("--parallel-contracts", action="store_true",
                        help="Overlap isolated Python contracts with selected checks; require both to succeed")
    parser.add_argument("--cargo-timings", action="store_true", help="Record Cargo unit timings for existing builds/tests")
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args()
    if args.preflight:
        check_temp_environment()
        print("Temporary environment prerequisites passed; not native proof.")
        return
    if args.receipt_dir and (args.pr or args.run):
        command = ['python3', str(Path(__file__).resolve()), '--base', args.base, '--head', args.head]
        if args.github_output:
            command += ['--github-output', str(args.github_output.resolve())]
        command += ['--suite', args.suite]
        if args.parallel_contracts:
            command += ['--parallel-contracts']
        if args.cargo_timings:
            command += ['--cargo-timings']
        command += [flag for flag, enabled in [('--pr', args.pr), ('--run', args.run), ('--online', args.online)] if enabled]
        label = ('changed-tree' if args.suite == 'all' else 'platform-' + args.suite) if args.run else 'pr-contracts'
        subprocess.run(['node', str(ROOT / 'enablement/verification/receipt.ts'),
                        '--suite', label,
                        '--output', str(args.receipt_dir.resolve()), '--', *command], cwd=ROOT, check=True)
        return
    selection = plan(args.base, args.head)
    if args.suite == 'rust':
        selection.update(root=True, engine=True, browser=False, focused=[])
    elif args.suite == 'browser':
        selection.update(root=False, engine=False, browser=True, focused=[])
    print(json.dumps(selection, sort_keys=True), flush=True)
    if args.github_output:
        with args.github_output.open("a") as output:
            for key in ("root", "engine", "browser"):
                output.write(f"{key}={str(selection[key]).lower()}\n")
    if args.pr:
        run_pr_checks()
    if args.run:
        run_checks(selection, offline=not args.online, parallel_contracts=args.parallel_contracts, cargo_timings=args.cargo_timings)


if __name__ == "__main__":
    if sys.argv[1:] == ['--contract-checks']:
        run_contract_checks()
        sys.exit(0)
    if len(sys.argv) > 1 and sys.argv[1] == "--hook-budget":
        sys.exit(hook_budget(float(sys.argv[2]), sys.argv[3:]))
    main()
