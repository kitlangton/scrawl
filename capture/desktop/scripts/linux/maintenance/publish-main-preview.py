#!/usr/bin/env python3
"""Publish checked main ancestors without superseding a newer checked preview.

The workflow serializes builds/publication without canceling the running release.
Ancestry, not completion time or version numbers, orders Main previews.
"""

import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys


def gh(*args):
    return subprocess.run(["gh", *args], check=True, capture_output=True, text=True).stdout


def release(repo, tag):
    result = subprocess.run(["gh", "release", "view", tag, "--repo", repo,
                             "--json", "isDraft,assets"], capture_output=True, text=True)
    if result.returncode:
        if "HTTP 404" in result.stderr or "release not found" in result.stderr.lower():
            return None
        raise RuntimeError(f"Could not inspect release {tag}: {result.stderr}")
    return json.loads(result.stdout)


def checked_sha(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-9a-f]{40}", value):
        raise RuntimeError(f"Malformed commit SHA: {value!r}")
    return value


def comparison(repo, base, tip):
    status = json.loads(gh("api", f"repos/{repo}/compare/{base}...{tip}"))["status"]
    if status not in ("ahead", "behind", "identical", "diverged"):
        raise RuntimeError(f"Unknown ancestry comparison: {status!r}")
    return status


def publication_needed(repo, sha):
    current = checked_sha(json.loads(gh("api", f"repos/{repo}/git/ref/heads/main"))["object"]["sha"])
    if comparison(repo, sha, current) not in ("ahead", "identical"):
        raise RuntimeError(f"Checked commit {sha} is not an ancestor of main {current}")

    # Fetch every page before deciding. A failed request or malformed history
    # aborts publication rather than guessing from a truncated release list.
    pages = json.loads(gh("api", f"repos/{repo}/releases?per_page=100", "--paginate", "--slurp"))
    if not isinstance(pages, list) or not pages or any(not isinstance(page, list) for page in pages):
        raise RuntimeError("Malformed paginated release history")
    newer = None
    for page in pages:
        for item in page:
            if not isinstance(item, dict) or not isinstance(item.get("tag_name"), str):
                raise RuntimeError("Malformed release history entry")
            if not item["tag_name"].startswith("linux-main-"):
                continue
            if not isinstance(item.get("draft"), bool) or not isinstance(item.get("prerelease"), bool):
                raise RuntimeError("Malformed Main release flags")
            if item["draft"] or not item["prerelease"]:
                continue
            published = checked_sha(item["tag_name"][len("linux-main-"):])
            if comparison(repo, published, current) not in ("ahead", "identical"):
                continue
            if comparison(repo, sha, published) in ("ahead", "identical"):
                newer = published
    if newer:
        print(f"Main preview {sha} not promoted: eligible preview {newer} is already published")
        return False
    return True


def main():
    repo = os.environ["GITHUB_REPOSITORY"]
    sha = checked_sha(os.environ["GITHUB_SHA"])
    if os.environ["GITHUB_REF"] != "refs/heads/main":
        raise RuntimeError("Main previews require refs/heads/main")
    tag = f"linux-main-{sha}"
    dist = Path("dist")
    images = sorted(dist.glob("*.AppImage"))
    expected = {f"Apparent-{os.environ['APPARENT_VERSION']}-linux-{arch}.AppImage"
                for arch in ("aarch64", "x86_64")}
    if {image.name for image in images} != expected:
        raise RuntimeError(f"Expected both checked AppImages: {expected}; found {[p.name for p in images]}")
    files = sorted(dist.iterdir())
    if {p.name for p in files} != expected | {name + ".sha256" for name in expected}:
        raise RuntimeError("Preview must contain exactly two AppImages and their checksums")
    digests = {}
    for image in images:
        digest = hashlib.sha256(image.read_bytes()).hexdigest()
        if (dist / (image.name + ".sha256")).read_text().strip() != f"{digest}  {image.name}":
            raise RuntimeError(f"Checksum mismatch: {image.name}")
        digests[image.name] = digest

    if not publication_needed(repo, sha):
        return

    existing = release(repo, tag)
    if existing:
        if not existing["isDraft"]:
            print(f"Published preview already exists: {tag}; leaving it unchanged")
            return
        # Recover a draft left by an interrupted attempt for this same commit.
        gh("release", "delete", tag, "--repo", repo, "--yes", "--cleanup-tag")

    # A draft is invisible to the updater until GitHub reports all four assets.
    try:
        gh("release", "create", tag, *(str(p) for p in files), "--repo", repo,
           "--target", sha, "--draft", "--prerelease", "--latest=false",
           "--title", f"Linux main {sha[:12]}",
           "--notes", "Automatic Linux preview from main; not a stable release.")
        draft = release(repo, tag)
        if draft is None or not draft["isDraft"]:
            raise RuntimeError("Draft release was not created privately")
        assets = {asset["name"]: asset for asset in draft["assets"]}
        if set(assets) != {p.name for p in files}:
            raise RuntimeError("Draft release assets are incomplete")
        for image in images:
            asset = assets[image.name]
            checksum = assets[image.name + ".sha256"]
            if (asset["state"] != "uploaded" or asset["digest"] != "sha256:" + digests[image.name]
                    or checksum["state"] != "uploaded"
                    or checksum["digest"] != "sha256:" + hashlib.sha256(
                        (dist / (image.name + ".sha256")).read_bytes()).hexdigest()):
                raise RuntimeError(f"Remote asset differs from checked build: {image.name}")
        if not publication_needed(repo, sha):
            return
        gh("release", "edit", tag, "--repo", repo, "--draft=false")
        public = release(repo, tag)
        if public is None or public["isDraft"]:
            raise RuntimeError("Release remains a draft after publication")
        print(f"Published checked Linux preview {tag} with both AppImages and checksums")
    finally:
        # An edit can succeed even when its readback fails. Never retract a public
        # good build; only private failed drafts are ours to clean up.
        partial = release(repo, tag)
        if partial and partial["isDraft"]:
            gh("release", "delete", tag, "--repo", repo, "--yes", "--cleanup-tag")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.CalledProcessError, OSError, KeyError, ValueError) as exc:
        print(f"Linux preview not published successfully: {exc}", file=sys.stderr)
        sys.exit(1)
