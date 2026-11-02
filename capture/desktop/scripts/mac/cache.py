#!/usr/bin/env python3
"""Portable, lock-verified crates.io download cache; never a build receipt."""
import argparse
import hashlib
import os
from pathlib import Path
import re
import stat
import sys
import tarfile
import tempfile
import tomllib

REGISTRY = 'index.crates.io-1949cf8c6b5b557f'
LIMIT = 512 * 1024 * 1024
COUNT = 10000


def allowed(lock):
    packages = tomllib.loads(lock.read_text())['package']
    return {f"{p['name']}-{p['version']}.crate": p['checksum']
            for p in packages
            if p.get('source') == 'registry+https://github.com/rust-lang/crates.io-index'
            and re.fullmatch(r'[A-Za-z0-9_.+-]+', p['name'])
            and re.fullmatch(r'[A-Za-z0-9_.+-]+', p['version'])}


def digest(data):
    return hashlib.sha256(data).hexdigest()


def export_cache(home, archive, lock):
    """Publish a new archive only; never overwrite an operator's file."""
    files = []
    total = 0
    root = home / 'registry' / 'cache' / REGISTRY
    for name, checksum in allowed(lock).items():
        path = root / name
        if not path.exists():
            continue
        if any(p.is_symlink() for p in (path, *path.parents)) or not path.is_file():
            raise ValueError('cache source contains symlink or special file')
        total += path.stat().st_size
        if total > LIMIT or len(files) >= COUNT:
            raise ValueError('cache exceeds bounded size/count')
        with path.open('rb') as source:
            data = source.read(LIMIT + 1)
        if len(data) > LIMIT:
            raise ValueError('source grew beyond size cap')
        if digest(data) != checksum:
            raise ValueError('source checksum mismatch: ' + name)
        files.append((name, data))
    if not files:
        return False
    # Exclusive creation and private mode, even under a permissive umask.
    fd = os.open(archive, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        import io
        with os.fdopen(fd, 'wb') as output, tarfile.open(fileobj=output, mode='w') as tar:
            for name, data in files:
                entry = tarfile.TarInfo(name)
                entry.size, entry.mode = len(data), 0o600
                tar.addfile(entry, io.BytesIO(data))
    except BaseException:
        archive.unlink()
        raise
    return True


def import_cache(home, archive, lock):
    """Validate everything before publishing a fresh dedicated Cargo home."""
    try:
        metadata = archive.lstat()
    except FileNotFoundError:
        return False
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > LIMIT + COUNT * 1024:
        raise ValueError('unsafe or oversized archive')
    if home.exists() or home.is_symlink():
        raise ValueError('destination must be a new dedicated Cargo home')
    checksums = allowed(lock)
    total = 0
    seen = set()
    # Same-filesystem staging allows atomic rename; no extraction API is used.
    with tempfile.TemporaryDirectory(prefix='.apparent-cache-', dir=home.parent) as tmp:
        stage = Path(tmp) / 'cargo'
        stage.mkdir(mode=0o700)
        root = stage / 'registry' / 'cache' / REGISTRY
        root.mkdir(parents=True, mode=0o700)
        for directory in (root.parent, root.parent.parent):
            directory.chmod(0o700)
        with tarfile.open(archive, mode='r:') as tar:
            for entry in tar:
                name = entry.name
                if (not entry.isfile() or name not in checksums or name in seen
                        or '/' in name or '\\' in name or entry.size < 0):
                    raise ValueError('unsafe, duplicate or non-lockfile archive entry')
                seen.add(name)
                total += entry.size
                if total > LIMIT or len(seen) > COUNT:
                    raise ValueError('cache exceeds bounded size/count')
                data = tar.extractfile(entry).read()
                if len(data) != entry.size or digest(data) != checksums[name]:
                    raise ValueError('archive checksum mismatch: ' + name)
                path = root / name
                with path.open('xb') as output:
                    path.chmod(0o600)
                    output.write(data)
        if not seen:
            return False
        # Destination must remain absent; operators must not run concurrent imports.
        if home.exists() or home.is_symlink():
            raise ValueError('destination appeared during import')
        stage.rename(home)
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('export', 'import'))
    parser.add_argument('--cargo-home', type=Path, required=True)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--lock', type=Path, default=Path('Cargo.lock'))
    args = parser.parse_args()
    try:
        hit = (export_cache if args.action == 'export' else import_cache)(
            args.cargo_home.absolute(), args.archive.absolute(), args.lock)
        print('cache hit' if hit else 'cache miss')
        return 0 if hit else 3
    except (OSError, ValueError, KeyError, tarfile.TarError) as error:
        print(f'cache rejected: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
