#!/usr/bin/env python3
"""Explicit suite entrypoints and advisory dependencies; never a CI skip decision."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

import verification as legacy

ROOT = legacy.ROOT
LANES = ('capture', 'engine-staged', 'connect-client', 'connect-web',
         'connect-service', 'public-site', 'desktop-ui')
ASSETS = {'public/site/public/assets/' + p for p in (
    'hosted/source-pond-v4.png', 'hosted/agent-pond-v4.png',
    'hosted/agent-reading-v4.png', 'fonts/HostGrotesk-Variable.ttf',
    'brand/apparent-wordmark.svg')}


def consumers(path):
    if path in ASSETS:
        return {'connect-web', 'public-site'}, 'Public asset consumed by Connect web'
    for prefix, lanes, reason in (
        ('connect/client/', {'connect-client', 'capture'}, 'client package and real application provider integration'),
        ('connect/web/', {'connect-web', 'public-site'}, 'browser package and Public composition'),
        ('connect/service/', {'connect-service', 'connect-client', 'connect-web', 'public-site', 'capture'}, 'service API and application/browser consumers'),
        ('public/site/', {'public-site'}, 'Public build, packaging and browser contracts'),
        ('shared/ui/', {'desktop-ui', 'connect-service', 'capture'}, 'shared Desktop and service recipient UI; Rust bridge integration'),
        ('capture/desktop/ui/frontend/', {'desktop-ui', 'capture'}, 'renderer/Electron and real Rust bridge integration'),
        ('capture/desktop/', {'capture', 'desktop-ui', 'engine-staged'}, 'Desktop native/Rust bridge and renderer consumers'),
        ('capture/engine/src/', {'engine-staged', 'capture'}, 'staged package is not production Engine coverage'),
        ('capture/', {'capture', 'engine-staged'}, 'whole application: Engine/Desktop/Agent reverse dependencies remain'),
        ('connect/', {'capture', 'connect-client', 'connect-service', 'connect-web', 'public-site'}, 'root application Connect integration'),
        ('shared/', set(LANES), 'shared contracts and build/native inputs'),
    ):
        if path.startswith(prefix):
            return lanes, reason
    return set(LANES), 'unclassified build/tooling/contract input'


def select(changes, history_known=True):
    """Changes are (status, path); additions/deletions include both rename sides."""
    mapping = []
    selected = set()
    if not history_known or not changes:
        selected.update(LANES)
        mapping.append({'path': None, 'status': None, 'lanes': sorted(LANES),
                        'reason': 'missing history or empty delta'})
    for status, path in sorted(set(changes)):
        lanes, reason = consumers(path)
        if status != 'M':
            lanes = set(LANES)
            reason = 'new/deleted/renamed/type-changed input; conservative coverage'
        selected.update(lanes)
        mapping.append({'path': path, 'status': status, 'lanes': sorted(lanes), 'reason': reason})
    return {'schema': 1, 'mode': 'advisory', 'executed': False,
            'selected': sorted(selected), 'inputs': mapping}


def plan(base, head):
    try:
        # Feature branches use the complete merge-base delta, not last push.
        ancestor = subprocess.run(['git', 'merge-base', base, head], cwd=ROOT,
                                  capture_output=True, text=True, check=True).stdout.strip()
        raw = subprocess.run(['git', 'diff', '--no-renames', '--name-status', '-z',
                              ancestor, head, '--'], cwd=ROOT, capture_output=True, check=True).stdout
        fields = raw.decode('utf-8').rstrip('\0').split('\0') if raw else []
        result = select(list(zip(fields[::2], fields[1::2])))
        result.update(base=ancestor, head=subprocess.run(['git', 'rev-parse', head], cwd=ROOT,
                      capture_output=True, text=True, check=True).stdout.strip())
        return result
    except (subprocess.CalledProcessError, UnicodeError, ValueError):
        return select([], False)


def summary(report):
    # JSON quoting plus HTML escaping prevents filenames from injecting Markdown.
    import html
    rows = ['### Advisory owner verification', '',
            'Proposed selection only; no tests executed by this report. Full platform suites remain required.', '',
            'Selected: ' + ', '.join(report['selected']), '', '<pre>']
    rows.extend(html.escape(json.dumps(row, sort_keys=True)) for row in report['inputs'])
    return '\n'.join(rows + ['</pre>', ''])


def run_lane(lane, online=False):
    """Manual explicit entrypoints; CI reporting never calls these."""
    mode = '--locked' if online else '--offline'
    jobs = __import__('os').environ.get('T3_BUILD_JOBS', '3')
    def run(args, directory='.'):
        subprocess.run(args, cwd=ROOT / directory, check=True)
    if lane in ('capture', 'engine-staged', 'connect-web'):
        legacy.run_checks({'root': lane == 'capture', 'engine': lane == 'engine-staged',
                           'browser': lane == 'connect-web', 'focused': []}, offline=not online)
    elif lane == 'connect-client':
        legacy.check_temp_environment()
        for command in ('build', 'test'):
            run(['cargo', command, '--manifest-path', 'connect/client/Cargo.toml', '-j', jobs, mode]
                + (['--', '--test-threads=2'] if command == 'test' else []))
    elif lane == 'connect-service':
        run(['npm', 'ci'], 'connect/service')
        run(['npm', 'test'], 'connect/service')
    elif lane == 'desktop-ui':
        directory = 'capture/desktop/ui/frontend'
        run(['npm', 'ci', '--no-audit', '--no-fund'], directory)
        run(['npm', 'test'], directory)
        run((['xvfb-run', '-a'] if sys.platform == 'linux' else []) + ['npm', 'run', 'test:electron'], directory)
    elif lane == 'public-site':
        run(['node', 'scripts/site-build/verify.mjs'], 'public/site')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--base', default='origin/main')
    parser.add_argument('--head', default='HEAD')
    parser.add_argument('--json', type=Path)
    parser.add_argument('--summary', type=Path)
    parser.add_argument('--lane', choices=LANES)
    parser.add_argument('--run', action='store_true')
    parser.add_argument('--online', action='store_true')
    args = parser.parse_args()
    if args.run:
        if not args.lane:
            parser.error('--run requires an explicit --lane; advisory selection cannot execute suites')
        run_lane(args.lane, args.online)
        return
    report = plan(args.base, args.head)
    encoded = json.dumps(report, sort_keys=True, indent=2) + '\n'
    print(encoded, end='')
    if args.json:
        args.json.write_text(encoded)
    if args.summary:
        with args.summary.open('a') as output:
            output.write(summary(report))


if __name__ == '__main__':
    main()
