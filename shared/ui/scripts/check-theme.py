#!/usr/bin/env python3
"""Ratchet legacy fixed colors; new frontend colors belong in shared tokens.

The baseline reserves existing artwork/demo/legacy literals, by path and count.
It grants no allowance to new files. Reduce it as old surfaces adopt tokens;
new exceptions require an explicit, reviewed baseline edit.
"""
from collections import Counter
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
BASELINE = Path(__file__).with_name('legacy-colors.json')
# Hex colors, CSS color functions, fixed Tailwind palettes and forced schemes.
COLOR = re.compile(r'#[\da-fA-F]{3,8}\b|\b(?:rgba?|hsla?|oklch|oklab|lab|lch)\([^)]*\)|\b(?:bg|text|border|ring|fill|stroke)-(?:white|black|(?:slate|gray|zinc|neutral|stone|red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-\d{2,3})\b|color-scheme\s*:\s*(?:light|dark)\s*[;}]')

NAMED = re.compile(r"\b(?:background(?:Color|-color)?|color|fill|stroke|borderColor|outlineColor|border-color|outline-color|accent-color)\s*:\s*['\"]?(?!var\b|url\b|none\b|transparent\b|currentColor\b|inherit\b|initial\b|unset\b|revert\b)([a-zA-Z]+)\b(?=\s*(?:[\x27\x22;}]|!important|$))")

def colors_in(source):
    return Counter(COLOR.findall(source) + ['named:' + name for name in NAMED.findall(source)])

def inventory(root=ROOT, owner=None):
    owners = (owner, 'shared') if owner else ('capture', 'connect', 'public', 'shared')
    try:
        git_root = subprocess.check_output(['git', 'rev-parse', '--show-toplevel'], cwd=root, stderr=subprocess.DEVNULL).decode().strip()
    except subprocess.CalledProcessError:
        git_root = ''
    if git_root and Path(git_root).resolve() == root.resolve():
        paths = subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard', '--', *owners], cwd=root).decode().split('\0')
    else:
        # Standalone source distribution: no dependency on a surrounding Git checkout.
        paths = [str(file.relative_to(root)) for owner in owners
                 if (root / owner).exists() for file in (root / owner).rglob('*')
                 if not any(part.startswith(('dist', '.')) or part == 'node_modules' for part in file.relative_to(root).parts)]
    result = {}
    for path in sorted(set(paths)):
        if not path or not path.startswith(tuple(owner + '/' for owner in owners)):
            continue
        file = root / path
        if file.suffix not in ('.css', '.tsx', '.ts', '.html', '.js', '.cjs') or not file.is_file():
            continue
        if any(part in ('test', 'tests', 'scripts', 'node_modules') for part in file.relative_to(root).parts):
            continue
        if path == 'shared/ui/src/tokens.css':
            continue
        colors = colors_in(file.read_text())
        if colors:
            result[path] = dict(sorted(colors.items()))
    return result

def violations(current, baseline):
    return [f'{path}: {color} ({count} > {baseline.get(path, {}).get(color, 0)})'
            for path, colors in current.items() for color, count in colors.items()
            if count > baseline.get(path, {}).get(color, 0)]

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--owner', choices=('public', 'connect', 'capture'),
                        help='Check this owner and shared inputs; default checks every frontend')
    args = parser.parse_args()
    failures = violations(inventory(owner=args.owner), json.loads(BASELINE.read_text()))
    if failures:
        print('New fixed frontend colors: use shared/ui/src/tokens.css semantic tokens.\n' + '\n'.join(failures), file=sys.stderr)
        sys.exit(1)
    print('Shared theme color contract passed.')
