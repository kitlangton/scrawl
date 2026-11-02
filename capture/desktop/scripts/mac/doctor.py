#!/usr/bin/env python3
"""Launch-free prerequisites for disposable Mac verification."""
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'enablement'))
from verification import check_temp_environment


def environment():
    env = os.environ.copy()
    env.setdefault('RUSTUP_TOOLCHAIN', 'stable')
    env.setdefault('TOOLCHAINS', 'MetalToolchain')
    # Match the Desktop CI Node major, rather than the vendor image default.
    workflow = (ROOT / '.github/workflows/verification-platform.yml').read_text()
    major = re.search(r"node-version: ['\"](\d+)['\"]", workflow)
    if not major:
        raise RuntimeError('Cannot determine Desktop CI Node major')
    paths = [str(Path.home() / '.cargo/bin')]
    for prefix in ('/opt/homebrew', '/usr/local'):
        paths += [f'{prefix}/opt/node@{major[1]}/bin',
                  f'{prefix}/opt/rustup/bin', f'{prefix}/bin']
    env['PATH'] = os.pathsep.join(paths + [env.get('PATH', '')])
    return env, major[1]


def run(argv, env, timeout=120):
    try:
        return subprocess.run(argv, env=env, check=True, text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              timeout=timeout).stdout.strip()
    except (OSError, subprocess.SubprocessError) as error:
        detail = getattr(error, 'stderr', '') or str(error)
        raise RuntimeError(f'{argv[0]} prerequisite failed: {detail}') from error


def doctor(env, major):
    if sys.platform != 'darwin':
        raise RuntimeError('Mac doctor requires Darwin; Linux simulation is not Mac proof')
    # Reuse the repository trust/socket validator, including Darwin limits.
    original = os.environ.get('TMPDIR')
    try:
        os.environ['TMPDIR'] = env.get('TMPDIR', '/tmp')
        check_temp_environment()
    finally:
        if original is None:
            os.environ.pop('TMPDIR', None)
        else:
            os.environ['TMPDIR'] = original
    for argv in (['rustc', '--version'], ['cargo', '--version'],
                 ['rustfmt', '--version'], ['npm', '--version'],
                 ['ffmpeg', '-version'], ['ffprobe', '-version'],
                 ['xcode-select', '-p'], ['xcrun', '--sdk', 'macosx', '--show-sdk-path'],
                 ['xcrun', '--sdk', 'macosx', 'metal', '--version']):
        print(f'OK {" ".join(argv)}: {run(argv, env).splitlines()[0]}', flush=True)
    version = run(['node', '--version'], env)
    if not version.startswith(f'v{major}.'):
        raise RuntimeError(f'Expected Desktop CI Node {major}.x, got {version}')
    print(f'OK Node {version}; TMPDIR validated', flush=True)


def provision(env, major):
    # No bootstrap shell downloads, sudo, shell-profile writes or Xcode selection.
    if sys.platform != 'darwin':
        raise RuntimeError('Provisioning requires Darwin')
    for tool in ('brew', 'xcodebuild'):
        if not shutil.which(tool, path=env['PATH']):
            raise RuntimeError(f'Preinstall {tool} using its official installer in the disposable image')
    packages = [f'node@{major}', 'ffmpeg']
    if not shutil.which('rustup', path=env['PATH']):
        packages.append('rustup')
    run(['brew', 'install', *packages], env, timeout=600)
    run(['rustup', 'toolchain', 'install', 'stable', '--component', 'rustfmt'], env, timeout=600)
    # Explicit per-process toolchain, without changing rustup defaults/overrides.
    env['RUSTUP_TOOLCHAIN'] = 'stable'
    # Avoid downloading an already usable component on every provision.
    try:
        run(['xcrun', '--sdk', 'macosx', 'metal', '--version'], env)
    except RuntimeError:
        run(['xcodebuild', '-downloadComponent', 'MetalToolchain'], env, timeout=600)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('doctor', 'provision', 'run'))
    parser.add_argument('--disposable', action='store_true', help='Explicitly designate this host disposable')
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    # REMAINDER keeps child flags opaque; accept designation before the action.
    if command[:1] == ['--']:
        command = command[1:]
    if args.action != 'doctor' and not args.disposable:
        parser.error('provision/run require --disposable before the action')
    if args.action == 'run' and not command:
        parser.error('run requires -- COMMAND [ARG ...]')
    if args.action != 'run' and command:
        parser.error('unexpected command arguments')
    try:
        if sys.platform != 'darwin':
            raise RuntimeError('Mac setup requires Darwin; no installation performed')
        env, major = environment()
        if args.action == 'doctor':
            doctor(env, major)
        else:
            # /private/tmp avoids macOS /tmp symlink; the existing validator
            # checks all ancestors before any provisioning or child command.
            with tempfile.TemporaryDirectory(prefix='am-', dir='/private/tmp') as tmp:
                env['TMPDIR'] = tmp
                old = os.environ.get('TMPDIR')
                try:
                    os.environ['TMPDIR'] = tmp
                    check_temp_environment()
                finally:
                    if old is None:
                        os.environ.pop('TMPDIR', None)
                    else:
                        os.environ['TMPDIR'] = old
                if args.action == 'provision':
                    provision(env, major)
                doctor(env, major)
                if args.action == 'run':
                    return subprocess.run(command, env=env).returncode
        return 0
    except RuntimeError as error:
        print(f'Mac setup: {error}', file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
