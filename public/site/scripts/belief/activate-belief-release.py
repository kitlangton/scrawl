"""Historical overlay protocol for synthetic releases; production uses Pages."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys


def inventory(root):
    files = []
    for file in root.rglob('*'):
        assert not file.is_symlink(), f'Unexpected symlink: {file}'
        if file.is_file():
            data = file.read_bytes()
            files.append({'path': str(file.relative_to(root)), 'bytes': len(data),
                          'sha256': hashlib.sha256(data).hexdigest()})
    return sorted(files, key=lambda f: f['path'])


def reject_retired_origin(root):
    if Path(root).resolve() == Path('/opt/presentations/releases'):
        raise ValueError('Clanker production activation and rollback are retired; use Pages')


def activate(manifest_path, root, rollback=False):
    reject_retired_origin(root)
    # Every publisher of this symlink must use this same lock, including rollback.
    # Atomic rename alone is not compare-and-swap; this serializes cooperating writers.
    with (root / '.apparent-site-publish.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return activate_locked(manifest_path, root, rollback)


def activate_locked(manifest_path, root, rollback):
    reject_retired_origin(root)
    manifest_path = Path(manifest_path).resolve()
    manifest = json.loads(manifest_path.read_text())
    link = root / 'apparent-site'
    baseline = Path(manifest['baseline']).resolve()
    candidate = Path(manifest['candidate']).resolve()
    assert baseline.parent == root and baseline.name.startswith('apparent-site-')
    assert candidate != baseline and not manifest_path.is_relative_to(candidate)
    before = sorted(manifest['before'], key=lambda f: f['path'])
    after = sorted(manifest['after'], key=lambda f: f['path'])
    html_hash = next(f['sha256'] for f in after if f['path'] == 'index.html')
    release = root / ('apparent-site-me-too-' + html_hash[:12])
    expected = release if rollback else baseline
    assert link.is_symlink() and link.resolve() == expected, 'Active release advanced'
    assert inventory(baseline) == before, 'Baseline changed'
    assert inventory(candidate) == after, 'Candidate changed since verification'
    for original in before:
        if original['path'] != 'index.html':
            assert next(f for f in after if f['path'] == original['path']) == original
    if not rollback:
        assert not release.exists(), 'Refuse to replace an existing release'
        shutil.copytree(candidate, release)
    assert inventory(release) == after, 'Staged bytes differ'
    assert link.resolve() == expected and inventory(baseline) == before, 'Baseline advanced before switch'
    target = baseline if rollback else release
    temporary = root / ('.belief-switch-' + html_hash[:12])
    assert not temporary.exists() and not temporary.is_symlink()
    temporary.symlink_to(target.name)
    os.replace(temporary, link)
    assert link.resolve() == target
    return {'active': str(target), 'rollback': str(baseline),
            'html_sha256': next(f['sha256'] for f in (before if rollback else after) if f['path'] == 'index.html'),
            'urls': manifest['urls']}


def main():
    result = activate(Path(sys.argv[1]), Path('/opt/presentations/releases'),
                      rollback=sys.argv[2:] == ['--rollback'])
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
