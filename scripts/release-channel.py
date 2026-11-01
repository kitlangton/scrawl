#!/usr/bin/env python3
"""Cut a beta build of Scrawl, or promote a tested build to stable.

Every build is a candidate with a plain version and its own build number: the app says
"0.3.14 (18)", never "beta". Beta is a property of the build's appcast item
(<sparkle:channel>beta</sparkle:channel>); promotion removes that element, so each build has exactly
one item and stable users get the same signed bytes beta users tested. GitHub access is REST only.
"""
import argparse
import base64
import fcntl
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import shlex
import shutil
import subprocess
import time
import uuid
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
STORE = Path.home() / ".local/share/scrawl-release"
REPO = "kitlangton/scrawl"
TAP = "kitlangton/homebrew-tap"
SITE = "https://scrawl.kitlangton.com"
RELEASE_HOST = "kits-shiny-computer"  # Pro: fleet role releaseHost
NS = "http://www.andymatuschak.org/xml-namespaces/sparkle"
CHANNEL = f"{{{NS}}}channel"
ET.register_namespace("sparkle", NS)


def run(*args, **kwargs):
    return subprocess.check_output([str(a) for a in args], text=True, **kwargs).strip()


def api(path, method="GET", data=None):
    args = ["gh", "api", path, "--method", method]
    if data is not None:
        args += ["--input", "-"]
    result = run(*args, input=json.dumps(data) if data is not None else None)
    return json.loads(result) if result else None


def paginated(path):
    pages = json.loads(run("gh", "api", f"{path}?per_page=100", "--paginate", "--slurp"))
    return [entry for page in pages for entry in page]


# Versions, tags and build numbers


def parse_version(value):
    match = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", value)
    if not match:
        raise ValueError(f"Versions are plain X.Y.Z, got {value!r}")
    return tuple(map(int, match.groups()))


def parse_tag(name):
    """`v0.3.14` → ("0.3.14", None); `v0.3.14-b18` → ("0.3.14", 18); anything else → None."""
    match = re.fullmatch(r"v(\d+\.\d+\.\d+)(?:-b([1-9]\d*))?", name)
    if not match:
        return None
    return match[1], int(match[2]) if match[2] else None


def label(version, build):
    return f"{version}-b{build}"


def bump(version):
    a, b, c = parse_version(version)
    return f"{a}.{b}.{c + 1}"


def choose_version(requested, tag_names, manifests):
    """The version the next build belongs to: the newest one in flight, or `requested`."""
    stable, candidates = set(), {m["version"] for m in manifests}
    for name in tag_names:
        parsed = parse_tag(name)
        if parsed:
            (stable if parsed[1] is None else candidates).add(parsed[0])
    current = max(stable | candidates, key=parse_version, default=None)
    if requested:
        if current and parse_version(requested) < parse_version(current):
            raise ValueError(f"--version {requested} is older than {current}")
        version = requested
    elif current is None:
        raise ValueError("No earlier release; pass --version")
    else:
        version = current
    if version in stable:
        raise ValueError(f"{version} is already stable; cut the next version with --version {bump(version)}")
    return version


def feed_items(xml):
    return ET.fromstring(xml).findall("./channel/item")


def build_number(item):
    return int(item.findtext(f"{{{NS}}}version"))


def item_channels(xml):
    """Build → channel (None for stable). A build may have only one item."""
    channels = {}
    for item in feed_items(xml):
        build = build_number(item)
        if build in channels:
            raise ValueError(f"The feed has more than one item for build {build}")
        channels[build] = item.findtext(CHANNEL)
    return channels


def next_build(xml, manifests, tag_names):
    builds = list(item_channels(xml)) + [m["build"] for m in manifests]
    builds += [parsed[1] for parsed in map(parse_tag, tag_names) if parsed and parsed[1]]
    return max([0] + builds) + 1


def promoted_feed(xml, build):
    """Remove `build`'s channel element, making that exact item stable. Text-level, so every other
    byte (signatures, deltas, other items) stays as generate_appcast wrote it."""
    channels = item_channels(xml)
    if build not in channels:
        raise ValueError(f"Build {build} is not in the feed")
    newer = [b for b, channel in channels.items() if channel is None and b > build]
    if newer:
        raise ValueError(f"Refusing to promote build {build} behind stable build {max(newer)}")
    if channels[build] is None:
        return xml
    if channels[build] != "beta":
        raise ValueError(f"Build {build} is on channel {channels[build]!r}, not beta")
    blocks = list(re.finditer(r"<item>.*?</item>", xml, re.S))
    target = [b for b in blocks if re.search(rf"<sparkle:version>{build}</sparkle:version>", b[0])]
    if len(target) != 1:
        raise ValueError(f"Could not find build {build}'s item")
    block = target[0]
    item, removed = re.subn(r"\s*<sparkle:channel>beta</sparkle:channel>", "", block[0])
    if removed != 1:
        raise ValueError(f"Build {build}'s item has no single beta channel element")
    result = xml[:block.start()] + item + xml[block.end():]
    if item_channels(result) != {**channels, build: None}:
        raise ValueError("Promotion changed more than one item")
    return result


# Files


def save(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(data, indent=2) + "\n")
    temporary.replace(path)


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def immutable_copy(source, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        if digest(source) != digest(destination):
            raise ValueError(f"Refusing to overwrite different bytes: {destination}")
    else:
        temporary = destination.with_suffix(destination.suffix + ".partial")
        shutil.copy2(source, temporary)
        temporary.replace(destination)


def download(url, destination):
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_suffix(destination.suffix + ".partial")
    run("curl", "--fail", "--silent", "--show-error", "--location", "--retry", "3", "--max-time", "1200",
        "--output", temporary, url)
    temporary.replace(destination)


def wait_for(path):
    # skulk owns waiting; no sleep loop.
    run("skulk", "poll", "--every", "60s", "--equals", "ready", "--deadline", "2h", "--wait",
        "--", f"if test -f {shlex.quote(str(path))}; then echo ready; else echo pending; fi")


def check_result(path):
    result = json.loads(path.read_text())
    if result.get("error"):
        raise RuntimeError(result["error"])


# Building and signing (GUI-session launchd jobs, for the login keychain)


def gui_build(manifest, folder):
    done = folder / "build-result.json"
    name = f"dev.kit.scrawl-release-{manifest['build']}"
    domain = f"gui/{os.getuid()}"
    if subprocess.run(["launchctl", "print", f"{domain}/{name}"], capture_output=True).returncode == 0:
        # An interrupted run left its job behind: let it finish, then reuse or retry its result.
        if not done.exists():
            wait_for(done)
        run("launchctl", "bootout", f"{domain}/{name}")
    if done.exists() and not json.loads(done.read_text()).get("error"):
        return
    done.unlink(missing_ok=True)
    script = folder / "build.sh"
    q = lambda value: shlex.quote(str(value))
    script.write_text(
        "#!/bin/bash\nset -uo pipefail\n"
        'export PATH="$HOME/bin:$HOME/.cargo/bin:$HOME/.bun/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"\n'
        f"cd {q(ROOT)}\n"
        f"export SCRAWL_VERSION={q(manifest['version'])} SCRAWL_BUILD_NUMBER={q(manifest['build'])}\n"
        f"export SCRAWL_SPARKLE_DIR={q(STORE / 'sparkle-2.9.4')} SCRAWL_DIST_DIR={q(folder / 'artifacts')}\n"
        f"bash scripts/release-app.sh > {q(folder / 'build.log')} 2>&1\n"
        "status=$?\n"
        f"if [ \"$status\" = 0 ]; then echo '{{}}'; else echo '{{\"error\":\"Build failed; see build.log\"}}'; fi > {q(done)}.tmp\n"
        f"mv {q(done)}.tmp {q(done)}\n"
    )
    plist = folder / "build.plist"
    plist.write_bytes(plistlib.dumps({"Label": name, "ProgramArguments": ["/bin/bash", str(script)], "RunAtLoad": True}))
    run("launchctl", "bootstrap", domain, plist)
    try:
        wait_for(done)
    finally:
        run("launchctl", "bootout", f"{domain}/{name}")
    check_result(done)


def appcast(manifest, action):
    """Ask Pro's dev.kit.scrawl-appcast job to add (`beta`) or promote (`promote`) this build."""
    request_path = STORE / "appcast-request.json"
    job = f"gui/{os.getuid()}/dev.kit.scrawl-appcast"
    if request_path.exists():
        previous = json.loads(request_path.read_text())
        previous_result = Path(previous["result"])
        if not previous_result.exists():
            # A caller exited after submitting. Finish that request first; kickstart leaves a
            # running job alone.
            run("launchctl", "kickstart", job)
            wait_for(previous_result)
        ok = not json.loads(previous_result.read_text()).get("error")
        if ok and previous.get("action") == action and previous.get("manifest") == manifest:
            return
    script = STORE / "appcast.sh"
    backup = STORE / "appcast-before-channels.sh"
    if script.exists() and not backup.exists():
        shutil.copy2(script, backup)
    shutil.copy2(ROOT / "scripts/appcast-job.sh", script)
    script.chmod(0o755)
    shutil.copy2(Path(__file__), STORE / "release-channel.py")
    result = STORE / f"appcast-result-{uuid.uuid4().hex}.json"
    save(request_path, {"manifest": manifest, "action": action, "result": str(result)})
    run("launchctl", "kickstart", job)
    wait_for(result)
    check_result(result)


def appcast_job():
    """Runs inside dev.kit.scrawl-appcast. Never deletes from updates/; restores the feed on error."""
    request = json.loads((STORE / "appcast-request.json").read_text())
    updates = STORE / "updates"
    feed = updates / "appcast.xml"
    backup = STORE / "feed-backups" / f"appcast-{time.strftime('%Y%m%d-%H%M%S')}-{uuid.uuid4().hex[:6]}.xml"
    try:
        m, action = request["manifest"], request["action"]
        before = feed.read_text()
        channels = item_channels(before)
        files = {p.name for p in updates.iterdir() if p.is_file()}
        backup.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(feed, backup)
        tool = STORE / "sparkle-2.9.4/bin/generate_appcast"
        key = run("security", "find-generic-password", "-s", "https://sparkle-project.org", "-a", "ed25519", "-w")
        if action == "beta":
            if "--channel" not in run(tool, "--help"):
                raise RuntimeError("Sparkle generate_appcast lacks --channel")
            # --versions touches only this build; --maximum-versions 0 keeps every older item and
            # archive (deltas need them). An archive whose SUPublicEDKey doesn't match the key
            # gets no signature, which the checks below catch.
            run(tool, "--ed-key-file", "-", "--versions", m["build"], "--channel", "beta",
                "--maximum-versions", "0", "--download-url-prefix", SITE + "/releases/", updates, input=key)
            expected = {**channels, m["build"]: channels.get(m["build"], "beta")}
        elif action == "promote":
            text = promoted_feed(before, m["build"])
            temporary = feed.with_suffix(".xml.tmp")
            temporary.write_text(text)
            temporary.replace(feed)
            expected = {**channels, m["build"]: None}
        else:
            raise ValueError(f"Unknown appcast action {action!r}")
        after = feed.read_text()
        if item_channels(after) != expected:
            raise RuntimeError(f"Feed channels changed unexpectedly: {item_channels(after)} != {expected}")
        missing = files - {p.name for p in updates.iterdir() if p.is_file()}
        if missing:
            raise RuntimeError(f"generate_appcast moved archives out of updates/: {sorted(missing)}")
        item = next(i for i in feed_items(after) if build_number(i) == m["build"])
        enclosure = item.find("enclosure")
        signature = enclosure.get(f"{{{NS}}}edSignature")
        if not signature or not enclosure.get("url", "").endswith("/" + m["zip"]):
            raise RuntimeError("The feed lacks a signed enclosure for this build's archive")
        if item.findtext(f"{{{NS}}}shortVersionString") != m["version"]:
            raise RuntimeError("The feed item's version differs from the build's")
        run(tool.parent / "sign_update", "--ed-key-file", "-", "--verify", updates / m["zip"], signature, input=key)
        save(Path(request["result"]), {})
    except Exception as error:
        if backup.exists():
            shutil.copy2(backup, feed)
        save(Path(request["result"]), {"error": str(error)})
        raise


# GitHub


def ensure_tag(tag, commit):
    refs = api(f"repos/{REPO}/git/matching-refs/tags/{tag}")
    ref = next((r for r in refs if r["ref"] == f"refs/tags/{tag}"), None)
    if ref and ref["object"]["sha"] != commit:
        raise ValueError(f"Tag {tag} already points to another commit")
    if not ref:
        api(f"repos/{REPO}/git/refs", "POST", {"ref": f"refs/tags/{tag}", "sha": commit})


def ensure_release(tag, name, body, prerelease):
    existing = next((r for r in paginated(f"repos/{REPO}/releases") if r["tag_name"] == tag), None)
    if existing:
        if existing["prerelease"] != prerelease or existing["draft"]:
            raise ValueError(f"Release {tag} exists with a different publication state")
        return existing
    return api(f"repos/{REPO}/releases", "POST", {
        "tag_name": tag, "name": name, "body": body, "prerelease": prerelease, "make_latest": "false",
    })


def upload(release, path, name):
    assets = api(f"repos/{REPO}/releases/{release['id']}/assets?per_page=100")
    existing = next((a for a in assets if a["name"] == name), None)
    expected = "sha256:" + digest(path)
    if existing:
        if existing.get("digest") != expected:
            raise ValueError(f"Published asset differs: {name}")
        return
    asset = json.loads(run("gh", "api", "--method", "POST", "-H", "Content-Type: application/octet-stream",
                           f"https://uploads.github.com/repos/{REPO}/releases/{release['id']}/assets?name={name}",
                           "--input", path))
    if asset.get("digest") not in (None, expected):
        raise ValueError(f"Uploaded asset digest differs: {name}")


def cask_text(text, token, version, sha256):
    if token == "scrawl":
        # Homebrew's Sparkle strategy ignores channels, so stable must skip beta items itself.
        text = text.replace("strategy :sparkle, &:short_version",
                            "strategy :sparkle do |items|\n"
                            "      items.select { |item| item.channel.nil? }.map(&:short_version)\n"
                            "    end")
    text = re.sub(r'  version "[^"]+"', f'  version "{version}"', text, count=1)
    return re.sub(r'  sha256 "[^"]+"', f'  sha256 "{sha256}"', text, count=1)


def cask_pr(manifest, stable):
    token = "scrawl" if stable else "scrawl@beta"
    version = manifest["version"] if stable else manifest["label"]
    path = f"Casks/{token}.rb"
    on_main = any(f["name"] == f"{token}.rb" for f in api(f"repos/{TAP}/contents/Casks"))
    # Until scrawl@beta is merged, the first beta updates its draft PR (#39) instead.
    branch = f"scrawl-{version}" if on_main else "scrawl-beta-channel"
    refs = api(f"repos/{TAP}/git/matching-refs/heads/{branch}")
    if not any(r["ref"] == f"refs/heads/{branch}" for r in refs):
        main = api(f"repos/{TAP}/git/ref/heads/main")
        api(f"repos/{TAP}/git/refs", "POST", {"ref": f"refs/heads/{branch}", "sha": main["object"]["sha"]})
    source = api(f"repos/{TAP}/contents/{path}?ref={branch}")
    original = base64.b64decode(source["content"]).decode()
    text = cask_text(original, token, version, manifest["sha256"][manifest["dmg"]])
    if text != original:
        api(f"repos/{TAP}/contents/{path}", "PUT", {"message": f"{token} {version}", "branch": branch,
            "sha": source["sha"], "content": base64.b64encode(text.encode()).decode()})
    body = (f"## What\nUpdate `{token}` to {version} (build {manifest['build']}).\n\n"
            "## How\nThe notarized DMG published by the Scrawl release script, with its SHA-256.\n\n"
            "## Risk\nApp update only.\n\n## Proof\nThe hash matches the archived, tested build.")
    prs = api(f"repos/{TAP}/pulls?head=kitlangton:{branch}&state=all")
    if not prs:
        pr = api(f"repos/{TAP}/pulls", "POST", {"title": f"{token} {version}", "head": branch, "base": "main", "body": body})
        print(pr["html_url"])
    elif prs[0]["state"] == "open":
        print(prs[0]["html_url"])


# The site

# Cloudflare Workers Assets rejects files over 25 MiB.
ASSET_LIMIT = 25 * 1024 * 1024
SITE_DOWNLOAD = re.compile(r"Scrawl-\d+\.\d+\.\d+(?:-b[1-9]\d*)?-(?:arm64\.dmg|arm64\.zip|windows-x64\.zip)")


def site_downloads(releases):
    """(name, url, digest) for the GitHub downloads the site serves: Mac DMGs and zips and Windows
    zips from our own `vX.Y.Z` / `vX.Y.Z-bN` releases. Other releases (linux-preview-N) reuse asset
    names across tags and ship AppImages over the size limit, so they stay on GitHub."""
    downloads = []
    for release in releases:
        if release["draft"] or not parse_tag(release["tag_name"]):
            continue
        for asset in release["assets"]:
            if SITE_DOWNLOAD.fullmatch(asset["name"]):
                downloads.append((asset["name"], asset["browser_download_url"], asset.get("digest")))
    return downloads


def deploy(manifest, folder, stable):
    public = ROOT / "site/public"
    assets = public / "releases"
    assets.mkdir(exist_ok=True)
    for name in [manifest["dmg"], manifest["zip"]]:
        immutable_copy(folder / name, assets / name)
        if stable:
            immutable_copy(folder / name, assets / name.replace(manifest["label"], manifest["version"]))
    # Workers Assets deploys a whole snapshot: hydrate every published download, so a fresh
    # checkout never drops one.
    for name, url, sha in site_downloads(paginated(f"repos/{REPO}/releases")):
        target = assets / name
        if not target.exists():
            download(url, target)
        if sha and "sha256:" + digest(target) != sha:
            raise ValueError(f"Asset checksum differs: {target}")
    feed = STORE / "updates/appcast.xml"
    for enclosure in ET.fromstring(feed.read_text()).iter("enclosure"):
        name = enclosure.get("url").rsplit("/", 1)[1]
        source = STORE / "updates" / name
        if source.exists():
            immutable_copy(source, assets / name)
        elif not (assets / name).exists():
            download(enclosure.get("url"), assets / name)
    shutil.copy2(feed, public / "appcast.xml")
    # The download buttons follow the newest stable item, whatever betas are newer.
    channels = item_channels(feed.read_text())
    newest = max(b for b, channel in channels.items() if channel is None)
    version = next(i for i in feed_items(feed.read_text()) if build_number(i) == newest).findtext(
        f"{{{NS}}}shortVersionString")
    for suffix in ["arm64.dmg", "arm64.zip", "windows-x64.zip"]:
        target = assets / f"Scrawl-{version}-{suffix}"
        if not target.exists():
            download(f"{SITE}/releases/{target.name}", target)
        shutil.copy2(target, assets / f"Scrawl-latest-{suffix}")
    too_big = [p.name for p in public.rglob("*") if p.is_file() and p.stat().st_size > ASSET_LIMIT]
    if too_big:
        raise ValueError(f"Over Workers Assets' 25 MiB limit; remove them from site/public: {too_big}")
    run("skulk", "run", "--slot", "heavy", "--",
        "bash", "-c", "cd site && bun install --frozen-lockfile && bun run build && bunx wrangler@latest deploy", cwd=ROOT)


# The two commands


def plan(args, xml, manifests, commit, tags):
    """The build to publish. `tags` maps GitHub tag names to commits."""
    if args.command == "promote":
        manifest = next((m for m in manifests if m["build"] == args.build), None)
        if not manifest or "sha256" not in manifest:
            raise ValueError(f"No finished beta build {args.build} in {STORE / 'builds'}")
        stable_commit = tags.get(f"v{manifest['version']}")
        if stable_commit and stable_commit != manifest["commit"]:
            raise ValueError(f"{manifest['version']} is already stable as another build")
        promoted_feed(xml, args.build)  # The live feed has it and nothing newer is stable.
        return manifest
    manifest = next((m for m in manifests if m["commit"] == commit), None)
    if manifest:
        promoted = tags.get(f"v{manifest['version']}") == commit or item_channels(xml).get(manifest["build"], "beta") is None
        if promoted:
            raise ValueError(f"{manifest['version']} (build {manifest['build']}) is already stable; commit something "
                             f"and cut the next version with --version {bump(manifest['version'])}")
        if args.version and args.version != manifest["version"]:
            raise ValueError(f"{commit[:9]} is already build {manifest['build']} of {manifest['version']}; "
                             "commit something to cut another build")
        return manifest
    version = choose_version(args.version, tags, manifests)
    build = next_build(xml, manifests, tags)
    name = label(version, build)
    return {"version": version, "build": build, "label": name, "commit": commit,
            "dmg": f"Scrawl-{name}-arm64.dmg", "zip": f"Scrawl-{name}-arm64.zip"}


def preflight(stable, commit, dry_run):
    problems = []
    if run("hostname").split(".")[0].lower() != RELEASE_HOST:
        problems.append("Run releases on Pro, the release host")
    archived = STORE / "updates/appcast.xml"
    generated = ROOT / "site/public/appcast.xml"
    for line in run("git", "status", "--porcelain", cwd=ROOT).splitlines():
        # A retry may find the feed it already copied into the site; nothing else.
        if line.endswith(" site/public/appcast.xml") and archived.exists() and generated.read_bytes() == archived.read_bytes():
            continue
        problems.append(f"Release checkout is not clean: {line}")
    if not stable:
        if not dry_run:
            run("git", "fetch", "--quiet", "origin", "main", cwd=ROOT)
        if subprocess.run(["git", "merge-base", "--is-ancestor", commit, "origin/main"], cwd=ROOT).returncode:
            problems.append("HEAD is not on origin/main")
    return problems


def execute(args):
    xml = run("curl", "--fail", "--silent", "--show-error", "--location", "--max-time", "30", SITE + "/appcast.xml")
    manifests = [json.loads(p.read_text()) for p in (STORE / "builds").glob("*/release.json")]
    commit = run("git", "rev-parse", "HEAD", cwd=ROOT)
    tags = {t["name"]: t["commit"]["sha"] for t in paginated(f"repos/{REPO}/tags")}
    stable = args.command == "promote"
    manifest = plan(args, xml, manifests, commit, tags)
    folder = STORE / "builds" / str(manifest["build"])
    print(json.dumps({"action": args.command, **manifest, "archive": str(folder)}, indent=2))
    problems = preflight(stable, commit, args.dry_run)
    if args.dry_run:
        for problem in problems:
            print(f"Would stop: {problem}")
        print("Would " + (
            f"tag v{manifest['version']} at {manifest['commit'][:9]}; publish the archived bytes as "
            f"Scrawl-{manifest['version']}-*; copy the Windows zip from v{manifest['label']}; "
            f"drop build {manifest['build']}'s beta channel in the feed; mark the release latest; "
            if stable else
            f"build, sign and notarize {manifest['label']}; tag v{manifest['label']} and publish a "
            "prerelease; add a beta feed item; ") + "deploy the site; open the cask PR.")
        return
    if problems:
        raise RuntimeError("; ".join(problems))
    archived_feed = STORE / "updates/appcast.xml"
    live = set(item_channels(xml))
    if not live <= set(item_channels(archived_feed.read_text())):
        raise RuntimeError(f"{archived_feed} lacks builds the live feed has; it is not the source of truth")
    path = folder / "release.json"
    save(path, manifest)  # Reserves the build number before building; failed builds keep theirs.
    if "sha256" not in manifest:
        gui_build(manifest, folder)
        for name in [manifest["dmg"], manifest["zip"]]:
            immutable_copy(folder / "artifacts" / name, folder / name)
        manifest["sha256"] = {name: digest(folder / name) for name in [manifest["dmg"], manifest["zip"]]}
        save(path, manifest)
    for name, sha in manifest["sha256"].items():
        if digest(folder / name) != sha:
            raise ValueError(f"Archived artifact changed: {name}")
    beta_tag = f"v{manifest['label']}"
    if stable:
        tag = f"v{manifest['version']}"
        ensure_tag(tag, manifest["commit"])
        release = ensure_release(tag, f"Scrawl {manifest['version']}",
                                 f"Build {manifest['build']}, promoted from {beta_tag} with identical signed artifacts.",
                                 prerelease=False)
        for name in [manifest["dmg"], manifest["zip"]]:
            upload(release, folder / name, name.replace(manifest["label"], manifest["version"]))
        beta = api(f"repos/{REPO}/releases/tags/{beta_tag}")
        platform = [a for a in beta["assets"]
                    if re.search(r"-(windows|linux)-", a["name"]) and not a["name"].endswith(".sha256")]
        if not any("-windows-" in a["name"] for a in platform):
            raise ValueError(f"Wait for the Windows workflow to attach its zip to {beta_tag}")
        for asset in platform:
            target = folder / asset["name"]
            if not target.exists():
                download(asset["browser_download_url"], target)
            if asset.get("digest") != "sha256:" + digest(target):
                raise ValueError(f"Platform artifact digest mismatch: {asset['name']}")
            name = asset["name"].replace(manifest["label"], manifest["version"])
            upload(release, target, name)
            checksum = folder / f"{name}.sha256"
            checksum.write_text(f"{digest(target)}  {name}\n")
            upload(release, checksum, checksum.name)
    else:
        ensure_tag(beta_tag, manifest["commit"])
        release = ensure_release(beta_tag, f"Scrawl {manifest['version']} beta (build {manifest['build']})",
                                 "Beta build. Turn on Settings › General › Get beta updates to receive these; "
                                 "report problems from the menu's Report a Problem….", prerelease=True)
        for name in [manifest["dmg"], manifest["zip"]]:
            upload(release, folder / name, name)
    immutable_copy(folder / manifest["zip"], STORE / "updates" / manifest["zip"])
    appcast(manifest, "promote" if stable else "beta")
    deploy(manifest, folder, stable)
    if stable:
        api(f"repos/{REPO}/releases/{release['id']}", "PATCH", {"make_latest": "true"})
    cask_pr(manifest, stable)
    print(f"Published {manifest['version']} (build {manifest['build']}) as {'stable' if stable else 'beta'}. "
          "Commit site/public/appcast.xml in an appcast PR; the cask PR awaits Kit's merge.")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    beta = sub.add_parser("beta", help="build, sign and publish the next build as a beta")
    beta.add_argument("--version", help="start a new version (X.Y.Z); default: the version in flight")
    beta.add_argument("--dry-run", action="store_true", help="read only: print the plan")
    promote = sub.add_parser("promote", help="publish a tested beta build as stable")
    promote.add_argument("build", type=int)
    promote.add_argument("--dry-run", action="store_true", help="read only: print the plan")
    sub.add_parser("appcast-job")
    args = parser.parse_args()
    if args.command == "appcast-job":
        return appcast_job()
    if args.command == "beta" and args.version:
        parse_version(args.version)
    if args.dry_run:
        return execute(args)
    STORE.mkdir(parents=True, exist_ok=True)
    with (STORE / "release.lock").open("w") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise SystemExit("Another release is running (release.lock)")
        execute(args)


if __name__ == "__main__":
    try:
        main()
    except (ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"error: {error}")
