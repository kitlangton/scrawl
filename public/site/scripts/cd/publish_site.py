"""Locked website publication. Never run this as root; grant only release-tree access."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import tempfile
import time
from urllib.request import Request, urlopen

PUBLIC = {
    'index.html', 'connect', 'hosted', 'assets', 'pond', 'team', 'feature-demos',
    'apparent-wordmark.svg', 'icon.svg', 'HostGrotesk-Variable.ttf',
    'social-preview-v1.png', 'Apparent-ghost-demo-v4-lilac.mp4',
    'Apparent-ghost-demo-v4-lilac-poster.jpg', 'robots.txt', 'sitemap.xml',
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def inventory(root):
    result = {}
    for path in root.rglob('*'):
        require(not path.is_symlink(), f'Symlink in artifact: {path}')
        require(path.is_dir() or path.is_file(), f'Special file: {path}')
        if path.is_file():
            result[str(path.relative_to(root))] = hashlib.sha256(path.read_bytes()).hexdigest()
    return result


def current_main():
    request = Request('https://api.github.com/repos/tryapparent/apparent/git/ref/heads/main',
                      headers={'Accept': 'application/vnd.github+json', 'User-Agent': 'apparent-cd'})
    with urlopen(request, timeout=20) as response:
        return json.load(response)['object']['sha']


def switch(link, target):
    # Random name avoids collisions with abandoned switch paths; lock owns publication.
    temporary = link.parent / ('.cd-switch-' + os.urandom(12).hex())
    try:
        temporary.symlink_to(target.name)
        os.replace(temporary, link)
    finally:
        temporary.unlink(missing_ok=True)


def verify_http(release):
    # Verify ordinary visitor URLs, not only cache-busted responses. Caddy must allow
    # Connect and its assets before CD is enabled. No account/auth requests are made.
    paths = ['index.html', 'connect/index.html', 'connect/device/index.html',
             'hosted/index.html', 'hosted/device/index.html', 'icon.svg']
    paths += [str(p.relative_to(release)) for p in (release / 'assets').rglob('*') if p.is_file()]
    for path in paths:
        url_path = '/' if path == 'index.html' else '/' + path.removesuffix('index.html')
        expected = (release / path).read_bytes()
        for attempt in range(4):
            try:
                with urlopen('https://app.tryapparent.com' + url_path, timeout=25) as response:
                    require(response.read() == expected, f'Public bytes differ: {url_path}')
                break
            except Exception:
                if attempt == 3:
                    raise
                time.sleep(3)


def publish(root, package, sha, expected, *, current_sha=current_main, verify=verify_http):
    root, package = Path(root).resolve(), Path(package)
    require(root != Path('/opt/presentations/releases'),
            'Clanker production publication is retired; use the main-branch Pages publisher')
    require(re.fullmatch('[0-9a-f]{40}', sha) is not None, 'Invalid commit SHA')
    require(package.is_dir() and not package.is_symlink(), 'Package must be a real directory')
    require(all(p.name in PUBLIC for p in package.iterdir()), 'Non-public package path')
    require((package / 'index.html').is_file(), 'Missing homepage')
    package_hashes = inventory(package)
    link = root / 'apparent-site'
    with (root / '.apparent-site-publish.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        require(link.is_symlink(), 'Active site must be a symlink')
        baseline = link.resolve(strict=True)
        require(str(baseline) == expected and baseline.parent == root, 'Active baseline advanced')
        before = inventory(baseline)
        require(all(before.get(p, h) == h for p, h in package_hashes.items()
                    if p.startswith('assets/')), 'Immutable asset name collision')
        require(current_sha() == sha, 'Candidate is not current main')
        release = Path(tempfile.mkdtemp(prefix='apparent-site-cd-' + sha[:12] + '-', dir=root))
        # mkdtemp is private by default; Caddy needs read/traverse after staging.
        shutil.copytree(baseline, release, dirs_exist_ok=True)
        for name in package_hashes:
            destination = release / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(package / name, destination)
            destination.chmod(0o644)
        after = inventory(release)
        require(all(after.get(p) == h for p, h in package_hashes.items()), 'Staged package differs')
        require(all(after.get(p) == h for p, h in before.items() if p not in package_hashes),
                'Preserved assets changed')
        require(inventory(package) == package_hashes, 'Package changed during staging')
        require(link.resolve() == baseline and inventory(baseline) == before, 'Baseline changed')
        require(current_sha() == sha, 'Main advanced during staging')
        for path in release.rglob('*'):
            if path.is_dir():
                path.chmod(0o755)
        release.chmod(0o755)
        receipt = {'sha': sha, 'active': str(release), 'previous': str(baseline),
                   'package': package_hashes, 'before': before, 'after': after}
        # Private receipt is outside the served tree and supports guarded rollback.
        receipt_path = root / ('.' + release.name + '.json')
        with receipt_path.open('x') as output:
            output.write(json.dumps(receipt, indent=2) + '\n')
        receipt_path.chmod(0o600)
        switch(link, release)
        try:
            verify(release)
            require(link.resolve() == release, 'Active site changed during verification')
        except Exception:
            # Do not undo a later publisher that ignores our shared lock.
            if link.resolve() == release:
                switch(link, baseline)
            raise
        return receipt


def rollback(root, receipt_path):
    root = Path(root).resolve()
    require(root != Path('/opt/presentations/releases'),
            'Clanker production rollback is retired; use Pages deployment rollback')
    with (root / '.apparent-site-publish.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        receipt = json.loads(Path(receipt_path).read_text())
        active, previous = Path(receipt['active']), Path(receipt['previous'])
        require(active.parent == root and previous.parent == root, 'Invalid rollback targets')
        link = root / 'apparent-site'
        require(link.is_symlink() and link.resolve() == active, 'Active release advanced')
        require(inventory(active) == receipt['after'], 'Active release changed')
        require(inventory(previous) == receipt['before'], 'Previous release changed')
        switch(link, previous)
        return {'active': str(link.resolve())}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', default='/opt/presentations/releases')
    sub = parser.add_subparsers(dest='command', required=True)
    deploy = sub.add_parser('publish')
    deploy.add_argument('package', type=Path)
    deploy.add_argument('sha')
    deploy.add_argument('expected')
    undo = sub.add_parser('rollback')
    undo.add_argument('receipt', type=Path)
    args = parser.parse_args()
    result = (publish(args.root, args.package, args.sha, args.expected)
              if args.command == 'publish' else rollback(args.root, args.receipt))
    print(json.dumps({key: result[key] for key in ('sha', 'active', 'previous') if key in result}, indent=2))
