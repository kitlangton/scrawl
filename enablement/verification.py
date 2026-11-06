#!/usr/bin/env python3
"""Conservative changed-tree verification shared by pre-push and Actions.

PR pushes run contract checks; main CI runs full platform verification before CD.
Explicit local --run retains conservative changed-tree checks.
Both sides of renames/deletions are included. Missing history broadens checks.
"""
import argparse
import base64
import hashlib
import json
import os
import re
import stat
import signal
import sys
from pathlib import Path
import subprocess
from contextlib import contextmanager
import selectors


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


DIRECTORY_WIDTH_CONTRACT_INPUTS = {
    ".githooks/pre-commit",
    "enablement/directory-width/check.py",
    "enablement/directory-width/README.md",
    "enablement/file-length/check.ts",
    "enablement/verification.py",
}


def directory_width_contract_selected(paths):
    """Select its existing integration suite only when its inputs changed."""
    if not paths:
        return True  # Unknown history stays conservative.
    return any(path in DIRECTORY_WIDTH_CONTRACT_INPUTS
               or path.startswith("enablement/directory-width/tests/")
               for path in paths)


CONNECT_CLIENT_TYPESCRIPT = "connect/delivery/client/"
CONNECT_CLIENT_TYPESCRIPT_CONFIG = {
    CONNECT_CLIENT_TYPESCRIPT + name
    for name in ("package.json", "package-lock.json", "tsconfig.json")
}

# App Shell foundation is an independent TypeScript package. Its complete
# verifier closure includes the shared contracts it consumes and canonical
# shared UI components; these paths do not feed the Rust application build.
APP_SHELL_ROOT = "composition/app/"
APP_SHELL_DELIVERY_ROOT = "composition/delivery/"
APP_SHELL_INPUTS = (APP_SHELL_ROOT, APP_SHELL_DELIVERY_ROOT, "shared/contracts/", "shared/ui/",
                    ".github/actions/delivery-contracts/", ".github/actions/delivery-component/")
APP_SHELL_WORKFLOW = ".github/workflows/stream-app-shell.yml"

RECORD_PACKAGE_ROOT = "record/"
RECORD_FRONTEND_ROOT = "capture/desktop/ui/frontend/"
# The standalone fixture consumer has no native/legacy source dependency.
# Unknown additions remain broad until their dependency closure is reviewed.
ONBOARDING_PACKAGE_ROOT = "onboarding/"
ONBOARDING_PACKAGE_INPUTS = {
    ONBOARDING_PACKAGE_ROOT + name for name in (
        ".gitignore", "README.md", "delivery.json", "package.json", "package-lock.json",
        "tsconfig.json", "tsconfig.test.json", "src/index.ts", "src/controller.ts",
        "src/model.ts", "src/react.tsx", "src/styles.css", "src/assets.d.ts",
        "src/assets/frog.png", "src/permissions/controller.ts", "src/permissions/react.tsx",
        "test/onboarding.test.ts", "test/permissions.test.ts", "test/consumer.test.ts",
        "vendor/record-0.1.0-afd73fb.tgz",
        "vendor/apparent-app-contracts-0.1.0.tgz", "src/factory.ts", "src/adoption.ts", "test/adoption.test.ts",
    )
}
# These can change the toolchain, assembly or transitive source closure. Keep
# broad integration checks when they change, while still running owner checks.
ONBOARDING_PACKAGE_BUILD_INPUTS = {
    ONBOARDING_PACKAGE_ROOT + name for name in (
        "package.json", "package-lock.json", "delivery.json", "tsconfig.json",
        "tsconfig.test.json", "vendor/record-0.1.0-afd73fb.tgz",
        "vendor/apparent-app-contracts-0.1.0.tgz",
    )
}


def onboarding_package_selected(paths):
    return not paths or any(path.startswith(ONBOARDING_PACKAGE_ROOT) for path in paths)


RECORD_UI_INPUTS = {
    RECORD_FRONTEND_ROOT + "pill/index.html",
}
RECORD_TYPESCRIPT_INPUTS = {
    RECORD_PACKAGE_ROOT + name for name in (
        "package.json", "package-lock.json", "tsconfig.json",
        "scripts/copy-ui-assets.ts", "scripts/build-ui.ts",
        "src/index.ts", "src/backend/engine/client.ts", "src/backend/workflow/controller.ts",
        "src/capabilities/index.ts", "src/capabilities/fixtures.ts",
        "src/ui/index.ts", "src/ui/recording-pill.tsx", "src/ui/UploadPocket.tsx",
        "src/ui/types.ts", "src/ui/style.css", "src/ui/styles.d.ts",
        "test/workflow.test.ts", "test/capabilities.test.ts", "test/package-smoke.ts",
    )
}
RECORD_PILL_INPUTS = RECORD_UI_INPUTS | {
    RECORD_FRONTEND_ROOT + "src/pill/" + name for name in (
        "main.tsx", "legacy-mount.tsx", "legacy.css", "bridge.d.ts",
    )
}
RECORD_PILL_INPUTS |= {
    RECORD_FRONTEND_ROOT + "tests/pill/" + name for name in (
        "host.cjs", "smoke.cjs", "protocol.test.cjs", "pinning.test.ts",
        "exceptions.smoke.ts", "shape.test.ts", "framing.cjs", "visibility.test.ts",
        "host-swap.smoke.ts",
    )
}


def record_typescript_selected(paths):
    """Select Record checks for its package and the retained pill consumer seam."""
    if not paths:
        return True  # Unknown history must include the independently built renderer.
    return any(
        (path.startswith(RECORD_PACKAGE_ROOT) and not path.endswith(".md"))
        or path.startswith(("shared/ui/", "shared/assets/fonts/"))
        or path in RECORD_UI_INPUTS
        or path.startswith(RECORD_FRONTEND_ROOT + "src/pill/")
        or path.startswith(RECORD_FRONTEND_ROOT + "tests/pill/")
        for path in paths
    )


# This new owner has no legacy Rust/build consumer. SDK changes remain broad,
# while also exercising its Agent consumer. Unknown/native Agent files stay broad.
def agent_tests_selected(paths):
    return any(path.startswith(("agent/", "engine/sdk/")) for path in paths)


def isolated_agent_input(path):
    return path.startswith("agent/") and (
        path in {"agent/.gitignore", "agent/LICENSE"}
        or Path(path).suffix in {".ts", ".json", ".md"}
    )


def app_shell_selected(paths):
    """Select the App Shell check for package, contract, UI or workflow inputs."""
    if not paths:
        return True  # Unknown history stays conservative.
    return any(path.startswith(APP_SHELL_INPUTS) or path == APP_SHELL_WORKFLOW for path in paths)


def connect_client_typescript_selected(paths):
    """Select the existing client package test for its full local input set."""
    if not paths:
        return True  # Unknown history stays conservative.
    return any(
        path in CONNECT_CLIENT_TYPESCRIPT_CONFIG
        or path.startswith(CONNECT_CLIENT_TYPESCRIPT + "src/")
        or (path.startswith(CONNECT_CLIENT_TYPESCRIPT + "test/")
            and "/" not in path[len(CONNECT_CLIENT_TYPESCRIPT + "test/"):]
            and path.endswith(".test.ts"))
        for path in paths
    )


PUBLIC_SITE_ROOT = "public/site/"
PUBLIC_SITE_BUILD_INPUTS = {
    PUBLIC_SITE_ROOT + name for name in (
        "index.html", "package.json", "bun.lock", "tsconfig.json", "vite.config.ts",
    )
}
PUBLIC_SITE_TOOLING_INPUTS = {
    PUBLIC_SITE_ROOT + "lib/public-assets.ts",
    PUBLIC_SITE_ROOT + "scripts/site-build/agent-files.ts",
    PUBLIC_SITE_ROOT + "scripts/site-build/feature-audit.ts",
    PUBLIC_SITE_ROOT + "scripts/site-build/prerender.ts",
    "shared/analytics/config.ts",
    "shared/analytics/browser.ts",
    "shared/ui/src/tokens.css",
    "shared/ui/scripts/check-theme.py",
    "shared/ui/scripts/check-theme.ts",
    "shared/ui/scripts/legacy-colors.json",
}
PUBLIC_CONNECT_SHARED_ASSETS = {
    "public/site/public/assets/" + path for path in (
        "hosted/source-pond-v4.png", "hosted/agent-pond-v4.png",
        "hosted/agent-reading-v4.png", "fonts/HostGrotesk-Variable.ttf",
        "brand/apparent-wordmark.svg",
    )
}
PUBLIC_CONNECT_SHARED_INPUTS = PUBLIC_CONNECT_SHARED_ASSETS | {
    "shared/analytics/config.ts", "shared/analytics/browser.ts", "shared/ui/src/tokens.css",
}


def public_site_build_selected(paths):
    """Select Public's existing non-browser package build for its real inputs."""
    if not paths:
        return True  # Feature-push contracts do not run the selected root suite.
    return any(
        path in PUBLIC_SITE_BUILD_INPUTS | PUBLIC_SITE_TOOLING_INPUTS
        or path.startswith((PUBLIC_SITE_ROOT + "src/", PUBLIC_SITE_ROOT + "public/"))
        for path in paths
    )


def select(paths, history_known=True):
    paths = sorted(set(paths))
    # Unknown/shared/build/security/lifecycle input is broad by default. Do not
    # infer safety from a commit message, extension, or a newly created owner.
    broad = not history_known or not paths
    engine = False
    browser = False
    public_site = False
    app_shell = False
    record = not history_known or record_typescript_selected(paths)
    delivery = False
    agent = agent_tests_selected(paths)
    onboarding_package = not history_known or onboarding_package_selected(paths)
    reasons = []
    for path in paths:
        if path in ONBOARDING_PACKAGE_INPUTS - ONBOARDING_PACKAGE_BUILD_INPUTS:
            continue
        if isolated_agent_input(path):
            continue
        if path in DOCS or (path.startswith("docs/") and path.endswith(".md")):
            continue
        if path in RECORD_TYPESCRIPT_INPUTS or path in RECORD_PILL_INPUTS:
            continue
        if path.startswith(APP_SHELL_INPUTS) or path == APP_SHELL_WORKFLOW:
            app_shell = True
            if path == APP_SHELL_WORKFLOW or path.startswith((".github/actions/delivery-contracts/", ".github/actions/delivery-component/")):
                continue
            # Shared UI also feeds existing consumers, so continue through
            # the ordinary selector for those paths.
            if path.startswith(APP_SHELL_ROOT) or path.startswith(APP_SHELL_DELIVERY_ROOT) or path.startswith("shared/contracts/"):
                if path.startswith(APP_SHELL_DELIVERY_ROOT):
                    delivery = True
                continue
        if path in PUBLIC_CONNECT_SHARED_INPUTS:
            # Shared browser sources and Public-owned assets have Connect
            # consumers; preserve broad root and browser checks as well.
            public_site = True
            browser = True
            broad = True
            reasons.append(path)
            continue
        if public_site_build_selected([path]):
            public_site = True
            continue
        # The standalone Engine package and reusable core are checked outside
        # the root app. Legacy capture runtime/bundle/native modules remain
        # root application inputs and retain the broad root checks.
        if path.startswith(("engine/core/", "engine/src/", "engine/tests/", "engine/delivery/")):
            engine = True
        elif path.startswith("capture/engine/"):
            if not path.startswith("capture/engine/docs/"):
                broad = True
                reasons.append(path)
        elif path in {"engine/Cargo.toml", "engine/Cargo.lock", "engine/core/Cargo.toml", "engine/core/Cargo.lock"}:
            engine = True
        elif path in {"engine/README.md", "engine/README-media.md", "engine/core/README.md"}:
            engine = True
        elif path in BROWSER_FILES:
            browser = True
        else:
            broad = True
            reasons.append(path)
    return {"root": broad, "engine": broad or engine, "browser": browser, "agent": agent,
            "app_shell": app_shell or not history_known or not paths,
            "record": record,
            "onboarding_package": onboarding_package,
            "delivery": delivery or not history_known or not paths,
            "public_site": public_site or not history_known or not paths,
            "focused": [], "paths": paths,
            "reason": "unknown history or empty delta" if not history_known or not paths
            else "broad inputs: " + ", ".join(reasons) if broad else "owner-local Agent delta" if agent else "document/package-test-only delta"}


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


def run_pr_checks(selection=None):
    subprocess.run(["node", "--experimental-strip-types", "--test", "enablement/file-length/check.test.ts"], cwd=ROOT, check=True)
    subprocess.run(["python3", "shared/ui/scripts/check-theme.py"], cwd=ROOT, check=True)
    paths = selection["paths"] if selection else []
    if (selection or {}).get("agent", agent_tests_selected(paths)):
        run_agent_tests()
    if (selection or {}).get("app_shell", app_shell_selected(paths)):
        run_app_shell_check()
    if (selection or {}).get("record", record_typescript_selected(paths)):
        run_record_typescript_check()
    if (selection or {}).get("onboarding_package", onboarding_package_selected(paths)):
        run_onboarding_package_check()
    if (selection or {}).get("delivery", delivery_tests_selected(paths)):
        run_delivery_tests()
    if connect_client_typescript_selected(paths):
        run_connect_client_typescript_tests()
    if directory_width_contract_selected(paths):
        subprocess.run(["python3", "-m", "unittest", "discover", "-s",
                        "enablement/directory-width/tests", "-p", "test_*.py"],
                       cwd=ROOT, check=True)
    for pattern in ("test_*push*.py", "test_verification.py", "test_release_gate.py",
                    "test_temp_preflight.py", "test_owner_lanes.py"):
        subprocess.run(["python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                        "-p", pattern], cwd=ROOT, check=True)
    frontend_root = "capture/desktop/ui/frontend/"
    onboarding_inputs = (
        frontend_root + "src/onboarding/",
        frontend_root + "src/shared/",
        frontend_root + "src/components/",
        frontend_root + "src/styles/",
    )
    onboarding_exact = {
        frontend_root + path for path in (
            "onboarding.html", "package.json", "package-lock.json", "vite.config.ts",
            "scripts/tsconfig.onboarding.json", "tsconfig.json", "src/style.css",
            "src/main.tsx", "src/base.css",
        )
    }
    if any(path == frontend_root + "src/onboarding/main.tsx"
           or path.startswith(onboarding_inputs) or path in onboarding_exact
           or path.startswith("shared/ui/")
           or path == "shared/assets/fonts/HostGrotesk-Variable.ttf" for path in paths):
        frontend = ROOT / "capture/desktop/ui/frontend"
        if not (ROOT / (frontend_root + "src/onboarding/main.tsx")).is_file():
            raise RuntimeError("Onboarding entry was deleted or renamed; update its focused check contract before pushing")
        if not all((frontend / item).is_file() for item in (
                "node_modules/typescript/bin/tsc", "node_modules/vite/bin/vite.js",
                "node_modules/@apparent/ui/package.json")):
            raise RuntimeError("Onboarding feedback needs installed dependencies: run npm ci in capture/desktop/ui/frontend, then retry the push; the hook never installs dependencies")
        subprocess.run(["npm", "run", "check:onboarding"], cwd=frontend, check=True)
    if (selection or {}).get("public_site", public_site_build_selected(paths)):
        run_public_site_build()


def run_app_shell_check():
    """Run the installed App Shell package check; hooks never install deps."""
    package = ROOT / "composition/app"
    controls = ("package.json", "package-lock.json", "tsconfig.json")
    missing = [name for name in controls if not (package / name).is_file()]
    if missing:
        raise RuntimeError("App Shell check inputs are missing or deleted: " + ", ".join(missing))
    manifest = json.loads((package / "package.json").read_text())
    dependencies = {**manifest.get("dependencies", {}), **manifest.get("devDependencies", {})}
    absent = [name for name in dependencies if not (package / "node_modules" / name / "package.json").is_file()]
    if absent:
        raise RuntimeError("App Shell check needs installed dependencies: run npm ci in composition/app; the hook never installs dependencies")
    subprocess.run(["npm", "run", "check"], cwd=package, check=True)


def run_onboarding_package_check():
    """Verify the standalone owner; never build the legacy frontend or install."""
    package = ROOT / ONBOARDING_PACKAGE_ROOT
    required = ONBOARDING_PACKAGE_INPUTS - {"onboarding/.gitignore", "onboarding/README.md"}
    missing = sorted(path for path in required if not (ROOT / path).is_file())
    if missing:
        raise RuntimeError("Onboarding package inputs missing or deleted: " + ", ".join(missing))
    try:
        manifest = json.loads((package / "package.json").read_text())
        lock = json.loads((package / "package-lock.json").read_text())
        locked = lock["packages"][""]
        for section in ("dependencies", "devDependencies"):
            if manifest.get(section, {}) != locked.get(section, {}):
                raise ValueError("manifest/lock mismatch")
        dependencies = {**manifest.get("dependencies", {}), **manifest.get("devDependencies", {})}
        if not dependencies or not all(manifest.get("scripts", {}).get(key) for key in ("typecheck", "test", "build")):
            raise ValueError("missing owner checks/dependencies")
    except (OSError, KeyError, TypeError, ValueError, AttributeError) as error:
        raise RuntimeError("Onboarding manifest/lock/checks are invalid; repair onboarding/package.json and package-lock.json") from error
    missing_dependencies = [name for name in dependencies if not (package / "node_modules" / name / "package.json").is_file()]
    if missing_dependencies:
        raise RuntimeError("Onboarding package checks need installed locked dependencies: run npm ci in onboarding; the hook never installs dependencies. Missing: " + ", ".join(sorted(missing_dependencies)))
    try:
        installed = json.loads((package / "node_modules/.package-lock.json").read_text())["packages"]
        for name in dependencies:
            key = "node_modules/" + name
            expected, actual = lock["packages"][key], installed[key]
            if expected.get("resolved", "").startswith("file:"):
                artifact = (package / expected["resolved"][5:]).resolve()
                if not artifact.is_relative_to((package / "vendor").resolve()) or not artifact.is_file():
                    raise ValueError("local dependency is not a contained vendor artifact")
                integrity = "sha512-" + base64.b64encode(hashlib.sha512(artifact.read_bytes()).digest()).decode("ascii")
                if integrity != expected.get("integrity"):
                    raise ValueError("local dependency integrity mismatch")
            if any(expected.get(field) != actual.get(field) for field in ("version", "resolved", "integrity")):
                raise ValueError("stale installed dependency: " + name)
            version = json.loads((package / key / "package.json").read_text())["version"]
            if version != expected["version"]:
                raise ValueError("stale installed package: " + name)
    except (OSError, KeyError, TypeError, ValueError) as error:
        raise RuntimeError("Onboarding installed lock/dependencies are stale or missing; run npm ci in onboarding before pushing (the hook never installs)") from error
    subprocess.run(["npm", "run", "typecheck"], cwd=package, check=True)
    # test builds the distributable before exercising controller and mounted UI.
    subprocess.run(["npm", "test"], cwd=package, check=True)


def delivery_tests_selected(paths):
    """Select pure Node delivery contracts for changes to their TypeScript owner."""
    if not paths:
        return True  # Unknown history stays conservative.
    return any(path.startswith(APP_SHELL_DELIVERY_ROOT) for path in paths)


def run_agent_tests():
    """Run the installed TypeScript owner suite; never install or invoke Cargo."""
    package = ROOT / "agent"
    controls = ("package.json", "package-lock.json", "tsconfig.json")
    if any(not (package / name).is_file() for name in controls):
        raise RuntimeError("Agent check inputs are missing or deleted; restore the owner package")
    dependencies = ("typescript/bin/tsc", "@types/node/package.json", "@apparent/engine-sdk/package.json")
    if any(not (package / "node_modules" / name).is_file() for name in dependencies):
        raise RuntimeError("Agent checks need installed locked dependencies: run npm ci in agent; the hook never installs dependencies")
    subprocess.run(["npm", "test"], cwd=package, check=True)


def run_delivery_tests():
    """Run delivery's dependency-free Node tests without installing packages."""
    tests = sorted((ROOT / APP_SHELL_DELIVERY_ROOT).glob("*.test.ts"))
    if not tests:
        raise RuntimeError("Delivery checks need composition/delivery/*.test.ts; the hook never installs dependencies")
    subprocess.run(["node", "--experimental-strip-types", "--test",
                    *[str(test.relative_to(ROOT)) for test in tests]], cwd=ROOT, check=True)


def run_connect_client_typescript_tests():
    package = ROOT / "connect/delivery/client"
    controls = ("package.json", "package-lock.json", "tsconfig.json")
    missing_controls = [name for name in controls if not (package / name).is_file()]
    if missing_controls:
        raise RuntimeError(
            "Connect client TypeScript package controls are missing or deleted: "
            + ", ".join(missing_controls)
            + "; restore them or update the pre-push contract before pushing"
        )
    dependencies = (
        "node_modules/typescript/bin/tsc",
        "node_modules/@types/node/package.json",
        "node_modules/age-encryption/package.json",
        "node_modules/@noble/ed25519/package.json",
    )
    missing_dependencies = [name for name in dependencies if not (package / name).is_file()]
    if missing_dependencies:
        raise RuntimeError(
            "Connect client TypeScript feedback needs installed locked dependencies; "
            "run `npm ci --no-audit --no-fund` in connect/delivery/client, then retry. "
            "The pre-push hook never installs dependencies. Missing: "
            + ", ".join(missing_dependencies)
        )
    subprocess.run(["npm", "test"], cwd=package, check=True)


def run_record_typescript_check():
    """Verify the owner package and its existing Desktop pill consumer without Cargo."""
    package = ROOT / "record"
    package_dependencies = (
        "node_modules/typescript/bin/tsc",
        "node_modules/@types/node/package.json",
        "node_modules/@types/react/package.json",
        "node_modules/@apparent/ui/package.json",
        "node_modules/lucide-react/package.json",
        "node_modules/esbuild/package.json",
        "node_modules/@tailwindcss/cli/package.json",
    )
    missing = [name for name in package_dependencies if not (package / name).is_file()]
    if missing:
        raise RuntimeError(
            "Record TypeScript feedback needs installed locked dependencies; run `npm ci --no-audit --no-fund` in record. The pre-push hook never installs dependencies. Missing: "
            + ", ".join(missing)
        )
    frontend = ROOT / "capture/desktop/ui/frontend"
    frontend_dependencies = (
        "node_modules/typescript/bin/tsc",
        "node_modules/vite/bin/vite.js",
        "node_modules/@apparent/record/package.json",
        "node_modules/@apparent/ui/package.json",
    )
    missing = [name for name in frontend_dependencies if not (frontend / name).is_file()]
    if missing:
        raise RuntimeError(
            "Record pill consumer feedback needs installed locked dependencies; run `npm ci --no-audit --no-fund` in capture/desktop/ui/frontend. The pre-push hook never installs dependencies. Missing: "
            + ", ".join(missing)
        )
    for command in (
        ["npm", "run", "typecheck"], ["npm", "test"], ["npm", "run", "build"],
        ["npm", "pack", "--dry-run"],
    ):
        subprocess.run(command, cwd=package, check=True)
    for command in (["npm", "run", "build"], ["npm", "test"]):
        subprocess.run(command, cwd=frontend, check=True)


def run_public_site_build():
    package = ROOT / "public/site"
    controls = ("package.json", "bun.lock", "tsconfig.json", "vite.config.ts", "index.html")
    missing_controls = [name for name in controls if not (package / name).is_file()]
    if missing_controls:
        raise RuntimeError(
            "Public site build inputs are missing or deleted: "
            + ", ".join(missing_controls)
            + "; restore them or update the pre-push contract before pushing"
        )
    try:
        manifest = json.loads((package / "package.json").read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise RuntimeError("Public site package.json must remain valid for pre-push build feedback") from error
    dependencies = {**manifest.get("dependencies", {}), **manifest.get("devDependencies", {})}
    missing_dependencies = [
        name for name in dependencies
        if not (package / "node_modules" / name / "package.json").is_file()
    ]
    if missing_dependencies:
        raise RuntimeError(
            "Public site feedback needs installed locked dependencies; run "
            "`bun install --frozen-lockfile` in public/site, then retry. "
            "The pre-push hook never installs dependencies. Missing: "
            + ", ".join(sorted(missing_dependencies))
        )
    subprocess.run(["npm", "run", "build"], cwd=package, check=True)


def run_contract_checks():
    subprocess.run(["python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                    "-p", "test_*push*.py"], cwd=ROOT, check=True)
    subprocess.run(["python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                    "-p", "test_verification.py"], cwd=ROOT, check=True)
    subprocess.run(["python3", "-m", "unittest", "discover", "-s", "enablement/tests",
                    "-p", "test_temp_preflight.py"], cwd=ROOT, check=True)


def cleanup_owned_processes(processes):
    """Reap every owned child without hiding the verification's primary failure."""
    primary = sys.exc_info()[1]
    errors = []
    for process in processes:
        if process is None:
            continue
        try:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            except PermissionError:
                # XNU can return EPERM for a group containing only zombies.
                # A reaped leader alone does not exclude live descendants.
                if sys.platform != "darwin" or process.poll() is None:
                    raise
                snapshot = subprocess.run(
                    ["/bin/ps", "-axo", "pgid=,stat="], check=True,
                    capture_output=True, text=True, timeout=5)
                for line in snapshot.stdout.splitlines():
                    fields = line.split()
                    if len(fields) != 2 or not fields[0].isdigit() or not fields[1][0].isalpha():
                        raise RuntimeError("invalid process-group cleanup snapshot")
                    if int(fields[0]) == process.pid and not fields[1].startswith("Z"):
                        raise
                # No live members: do not retry a signal using an old group ID.
        except Exception as error:
            errors.append(error)
        finally:
            try:
                if process.poll() is None:
                    # Even when group cleanup fails, still reap our direct child.
                    process.kill()
                process.wait(timeout=5)
            except Exception as error:
                errors.append(error)
    if errors:
        detail = "; ".join(f"{type(error).__name__}: {error}" for error in errors)
        if primary is not None:
            if hasattr(primary, "add_note"):
                primary.add_note("Owned process cleanup failed: " + detail)
            print("Owned process cleanup failed: " + detail, file=sys.stderr, flush=True)
        else:
            raise RuntimeError("Owned process cleanup failed: " + detail) from errors[0]


def overlap_cargo_test_and_build(test_command, build_command):
    """Run the ordinary app build as soon as Cargo reports test compilation done."""
    test_process = build_process = None
    cancelled = None
    def _signal(signum, _frame):
        nonlocal cancelled
        cancelled = 128 + signum
    previous = {sig: signal.signal(sig, _signal)
                for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT)}
    test_status = build_status = None
    saw_success = False
    selector = None
    buffer = b""
    stream_open = False
    try:
        test_process = subprocess.Popen(test_command, cwd=ROOT, stdout=subprocess.PIPE,
                                        stderr=None, bufsize=0, start_new_session=True)
        if cancelled is not None:
            raise SystemExit(cancelled)
        os.set_blocking(test_process.stdout.fileno(), False)
        selector = selectors.DefaultSelector()
        selector.register(test_process.stdout, selectors.EVENT_READ)
        stream_open = True
        while stream_open or test_process.poll() is None or (build_process is not None and build_process.poll() is None):
            if cancelled is not None:
                raise SystemExit(cancelled)
            # Poll both processes while waiting for output: test stdout may be
            # quiet during a long compile and must not hide an early build error.
            for process, name in ((test_process, "test"), (build_process, "build")):
                if process is not None and process.poll() is not None:
                    status = process.returncode
                    if name == "test": test_status = status
                    else: build_status = status
                    if status:
                        # Preserve diagnostics already available without waiting
                        # for a potentially quiet test pipe.
                        if buffer:
                            print(buffer.decode(errors="replace"), end="", flush=True)
                            buffer = b""
                        try:
                            while True:
                                tail = os.read(test_process.stdout.fileno(), 65536)
                                if not tail: break
                                print(tail.decode(errors="replace"), end="", flush=True)
                        except BlockingIOError:
                            pass
                        raise subprocess.CalledProcessError(status, process.args)
            for key, _ in selector.select(timeout=0.05):
                chunk = os.read(key.fileobj.fileno(), 65536)
                if not chunk:
                    selector.unregister(key.fileobj)
                    stream_open = False
                    if buffer:
                        print(buffer.decode(errors="replace"), end="", flush=True)
                        buffer = b""
                    continue
                buffer += chunk
                lines = buffer.split(b"\n")
                buffer = lines.pop()
                for raw_line in lines:
                    line = raw_line.decode(errors="replace") + "\n"
                    if not line.strip():
                        print(line, end="", flush=True)
                        continue
                    try:
                        event = json.loads(line)
                    except json.JSONDecodeError:
                        print(line, end="", flush=True)
                        continue
                    if not isinstance(event, dict) or "reason" not in event:
                        print(line, end="", flush=True)
                        continue
                    if event.get("reason") == "compiler-message":
                        rendered = event.get("message", {}).get("rendered")
                        if rendered:
                            print(rendered, end="", flush=True)
                    elif event.get("reason") == "build-finished":
                        if event.get("success") is not True:
                            raise RuntimeError("Cargo test build-finished event reported failure")
                        if not saw_success:
                            saw_success = True
                            print("Root test compilation complete; starting required app build alongside test execution",
                                  flush=True)
                            build_process = subprocess.Popen(build_command, cwd=ROOT,
                                                             stdout=None, stderr=None,
                                                             start_new_session=True)
                            if cancelled is not None:
                                raise SystemExit(cancelled)
                    elif event.get("reason") not in ("compiler-artifact", "build-script-executed"):
                        print(line, end="", flush=True)
        if test_status is None:
            test_status = test_process.wait()
        if not saw_success:
            raise RuntimeError("Cargo test did not report build-finished success")
        if build_process is None:
            raise RuntimeError("Cargo app build was not started")
        build_status = build_process.wait()
        if build_status:
            raise subprocess.CalledProcessError(build_status, build_process.args)
        if test_status:
            raise subprocess.CalledProcessError(test_status, test_process.args)
        print("Root test harness and required app build both succeeded", flush=True)
    finally:
        try:
            cleanup_owned_processes((test_process, build_process))
        finally:
            try:
                if selector is not None:
                    selector.close()
                if test_process is not None and test_process.stdout is not None:
                    test_process.stdout.close()
            finally:
                for sig, handler in previous.items():
                    signal.signal(sig, handler)


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
        try:
            cleanup_owned_processes((process,))
        finally:
            for sig, handler in previous.items():
                signal.signal(sig, handler)


def run_checks(selection, offline=True, parallel_contracts=False, cargo_timings=False,
               overlap_root_build=False):
    if selection["root"] or selection["engine"]:
        check_temp_environment()
    subprocess.run(["python3", "shared/ui/scripts/check-theme.py"], cwd=ROOT, check=True)
    if parallel_contracts:
        with overlapping_contracts():
            run_selected_checks(selection, offline, cargo_timings, overlap_root_build)
    else:
        run_contract_checks()
        run_selected_checks(selection, offline, cargo_timings, overlap_root_build)


def run_selected_checks(selection, offline=True, cargo_timings=False, overlap_root_build=False):
    if selection.get("onboarding_package"):
        run_onboarding_package_check()
    if selection.get("record"):
        run_record_typescript_check()
    if selection.get("agent"):
        run_agent_tests()
    if selection.get("app_shell"):
        run_app_shell_check()
    if selection.get("delivery"):
        run_delivery_tests()
    cargo = ["cargo"]
    if subprocess.run(["sh", "-c", "command -v taskpolicy"], capture_output=True).returncode == 0:
        cargo = ["taskpolicy", "-c", "utility", "cargo"]
    jobs = os.environ.get("T3_BUILD_JOBS", "3")
    dependency_mode = "--offline" if offline else "--locked"
    diagnostics = ["--timings"] if cargo_timings else []
    # The standalone core and provider are smaller consumers than the root app.
    # Run both first when selected so package-local compiler or test failures
    # stop before the broader application compile. Legacy capture code remains
    # in the root app and still selects those checks independently.
    if selection["engine"]:
        for manifest in ("engine/core/Cargo.toml", "engine/Cargo.toml"):
            for command in ("build", "test"):
                args = cargo + [command, "--manifest-path", manifest, "-j", jobs, dependency_mode] + diagnostics
                if command == "test":
                    args += ["--", "--test-threads=2"]
                subprocess.run(args, cwd=ROOT, check=True)
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
            test_command = cargo + ["test", "-j", jobs, dependency_mode] + diagnostics
            test_command += (["--message-format=json-render-diagnostics"] if overlap_root_build else [])
            test_command += ["--", "--test-threads=2"]
            build_command = cargo + ["build", "-j", jobs, dependency_mode] + diagnostics
            if overlap_root_build:
                overlap_cargo_test_and_build(test_command, build_command)
            else:
                subprocess.run(test_command, cwd=ROOT, check=True)
        # Keep the preflight test harness warm before building the distinct app
        # binary required by subsequent bridge, packaging and native checks.
        if not (overlap_root_build and not selection.get("focused")):
            subprocess.run(build_command if not selection.get("focused") else
                           cargo + ["build", "-j", jobs, dependency_mode] + diagnostics,
                           cwd=ROOT, check=True)
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
        try:
            cleanup_owned_processes((process,))
        finally:
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
    parser.add_argument("--overlap-root-build", action="store_true",
                        help="Overlap the broad root Cargo test harness with the app build after Cargo signals build completion")
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
        if args.overlap_root_build:
            command += ['--overlap-root-build']
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
        selection.update(root=True, engine=True, browser=False, app_shell=False, agent=False, record=False, onboarding_package=False, focused=[])
    elif args.suite == 'browser':
        selection.update(root=False, engine=False, browser=True, app_shell=False, agent=False, record=False, onboarding_package=False, focused=[])
    print(json.dumps(selection, sort_keys=True), flush=True)
    if args.github_output:
        with args.github_output.open("a") as output:
            for key in ("root", "engine", "browser", "app_shell"):
                output.write(f"{key}={str(selection[key]).lower()}\n")
    if args.pr:
        run_pr_checks(selection)
    if args.run:
        run_checks(selection, offline=not args.online, parallel_contracts=args.parallel_contracts,
                   cargo_timings=args.cargo_timings, overlap_root_build=args.overlap_root_build)


if __name__ == "__main__":
    if sys.argv[1:] == ['--contract-checks']:
        run_contract_checks()
        sys.exit(0)
    if len(sys.argv) > 1 and sys.argv[1] == "--hook-budget":
        sys.exit(hook_budget(float(sys.argv[2]), sys.argv[3:]))
    main()
