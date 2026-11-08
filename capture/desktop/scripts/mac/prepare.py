#!/usr/bin/env python3
"""Launch-free admission for the existing device-free Darwin Swift regression suite."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import sys

ENTRY = 'capture/engine/native/tests/live-transcribe.py'
INPUTS = (ENTRY, 'capture/engine/native/tests/live-transcribe.swift',
          'capture/engine/native/LiveTranscribe.swift', 'capture/engine/native/Media.swift',
          'shared/native/Info.plist')


def git(root, *args):
    return subprocess.check_output(['git', '-C', str(root), *args], stderr=subprocess.PIPE)


def prepare(root, candidate):
    if not re.fullmatch('[0-9a-f]{40}', candidate):
        raise ValueError('candidate must be a full lowercase commit SHA')
    head = git(root, 'rev-parse', 'HEAD').decode().strip()
    if head != candidate:
        raise ValueError('stale candidate: checkout HEAD differs from requested commit')
    if git(root, 'status', '--porcelain', '--untracked-files=normal'):
        raise ValueError('candidate checkout must be clean (including untracked files)')
    flags = git(root, 'ls-files', '-v').decode().splitlines()
    if any(line[0].islower() or line.startswith('S ') for line in flags):
        raise ValueError('hidden index flags prevent candidate verification')
    inputs = []
    for name in INPUTS:
        path = root / name
        if path.is_symlink() or not path.is_file():
            raise ValueError(f'missing or symlink input: {name}')
        data = path.read_bytes()
        if data != git(root, 'show', f'{head}:{name}'):
            raise ValueError(f'input differs from candidate: {name}')
        inputs.append({'path': name, 'bytes': len(data),
                       'sha256': hashlib.sha256(data).hexdigest()})
    compile((root / ENTRY).read_bytes(), ENTRY, 'exec')
    plistlib.loads((root / INPUTS[-1]).read_bytes())
    # The fixed suite generates PCM internally. It has no request-ID or external media input.
    if head != git(root, 'rev-parse', 'HEAD').decode().strip() or git(root, 'status', '--porcelain'):
        raise ValueError('candidate changed during preparation')
    return {'schema': 1, 'status': 'PREPARED_ONLY', 'commit': head,
            'tree': git(root, 'rev-parse', 'HEAD^{tree}').decode().strip(),
            'scenario': 'live-transcribe', 'inputs': inputs,
            'command': ['python3', ENTRY], 'working_directory': '.',
            'fixture': 'sibling Swift test source; PCM generated in memory',
            'request_ids': 'not applicable: no capture-control requests',
            'guest_requirements': ['Darwin', 'Python 3', 'xcrun with Swift 6.2+',
                                   'macOS SDK: Speech, AVFoundation, CoreGraphics',
                                   'private writable OS temporary directory'],
            'gates': {name: 'UNRUN' for name in
                      ('guest_capabilities', 'darwin_compile', 'native_suite',
                       'installed_app', 'permissions', 'gui', 'media_acceptance')},
            'scope': 'Device-free converter/source/completion regression only; no platform parity proof'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=Path(__file__).resolve().parents[4])
    parser.add_argument('--candidate', required=True, help='exact clean checkout commit SHA')
    parser.add_argument('--receipt', required=True, type=Path, help='new file in existing private directory outside checkout')
    args = parser.parse_args()
    try:
        root = args.repo.resolve()
        output = args.receipt.absolute()
        parent = output.parent.resolve(strict=True)
        if parent.is_relative_to(root) or parent.stat().st_mode & 0o077:
            raise ValueError('receipt requires a private directory outside checkout (mode 0700)')
        receipt = prepare(root, args.candidate)
        # Exclusive creation preserves previous evidence; never execute the scenario here.
        fd = os.open(parent / output.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'w') as stream:
            json.dump(receipt, stream, indent=2)
            stream.write('\n')
    except (ValueError, OSError, SyntaxError, subprocess.CalledProcessError) as error:
        parser.exit(1, f'prepare: {error}\n')


if __name__ == '__main__':
    main()
