#!/usr/bin/env python3
"""Admit an exact main commit only after its publication-platform checks pass.

Used from trusted main by publication workflows, including manual/tag releases.
A successful PR check or an older selector-only workflow cannot admit a release.
"""
import argparse
import json
import os
import re
import subprocess
from pathlib import Path
import xml.etree.ElementTree as ET

WORKFLOWS = {
    'linux': '.github/workflows/verification.yml',
    'macos': '.github/workflows/verification-mac.yml',
}
REQUIRED = {
    platform: {'verify / select', f'verify / desktop-ui ({runner})',
               f'verify / rust ({runner})', f'verify / connect-client ({runner})',
               f'verify / connect-web ({runner})',
               'verify / Release verification'}
    for platform, runner in [('linux', 'ubuntu-24.04'), ('macos', 'macos-15-intel')]
}


def api(path):
    return json.loads(subprocess.check_output(['gh', 'api', path], text=True))


def pages(path, fetch):
    result = []
    for page in range(1, 101):
        data = fetch(f'{path}{"&" if "?" in path else "?"}per_page=100&page={page}')
        key = 'jobs' if '/jobs' in path else 'workflow_runs'
        result.extend(data[key])
        if len(result) >= data['total_count']:
            return result
        if not data[key]:
            break
    raise ValueError('Incomplete verification history; refusing publication')


def verified_run(repo, sha, platform, run_id, fetch):
    prefix = f'repos/{repo}'
    workflow = WORKFLOWS[platform]
    if run_id:
        runs = [fetch(f'{prefix}/actions/runs/{run_id}')]
    else:
        filename = workflow.rsplit('/', 1)[1]
        query = f'head_sha={sha}&branch=main&event=push' if platform == 'linux' else 'branch=main'
        runs = pages(f'{prefix}/actions/workflows/{filename}/runs?{query}', fetch)
    # Never accept a different commit, fork, workflow, PR/manual run or old attempt.
    for candidate in sorted(runs, key=lambda run: run['id'], reverse=True):
        run = fetch(f'{prefix}/actions/runs/{candidate["id"]}')
        follower = platform == 'macos' and run['event'] == 'workflow_run'
        if not (run['repository']['full_name'] == repo
                and run['head_repository']['full_name'] == repo
                and run['path'] == workflow
                and (follower or (run['head_sha'] == sha and run['event'] == 'push'))
                and run['head_branch'] == 'main'
                and run['status'] == 'completed' and run['conclusion'] == 'success'):
            continue
        jobs = pages(f'{prefix}/actions/runs/{run["id"]}/attempts/{run["run_attempt"]}/jobs', fetch)
        successful = {job['name'] for job in jobs
                      if job['status'] == 'completed' and job['conclusion'] == 'success'}
        required = REQUIRED[platform]
        if follower:
            # workflow_run's head_sha can be newer than the tested candidate.
            # Trusted workflow code names its terminal job with the exact checkout.
            required = (required - {'verify / Release verification'}) | {
                f'verify / Release verification ({sha})'}
        if required <= successful:
            return
    raise ValueError(f'No successful {platform} main verification for this exact commit')


def admit(repo, sha, run_id=None, fetch=api, platform='linux'):
    if platform not in WORKFLOWS:
        raise ValueError('Expected linux or macos publication platform')
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repo):
        raise ValueError('Invalid repository')
    if not re.fullmatch(r'[0-9a-f]{40}', sha):
        raise ValueError('Expected an exact commit SHA')
    if run_id is not None and not str(run_id).isdigit():
        raise ValueError('Invalid verification run ID')
    # Linux also covers shared/browser contracts. Mac publication retains that
    # receipt plus its own native platform checks. No Mac lookup blocks Linux.
    platforms = ['linux'] if platform == 'linux' else ['linux', 'macos']
    for required in platforms:
        verified_run(repo, sha, required, run_id if required == platform else None, fetch)
    status = fetch(f'repos/{repo}/compare/{sha}...main')['status']
    if status not in ('ahead', 'identical'):
        raise ValueError('Verified commit is not on main')
    return sha


def publication_needed(repo, sha, published, fetch=api):
    """Call under the publisher's concurrency lock, immediately before mutation."""
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repo):
        raise ValueError('Invalid repository')
    for value in (sha, published):
        if value is not None and not re.fullmatch(r'[0-9a-f]{40}', value):
            raise ValueError('Unknown publication revision; refusing to guess ordering')
    if fetch(f'repos/{repo}/compare/{sha}...main')['status'] not in ('ahead', 'identical'):
        raise ValueError('Candidate is no longer on main')
    if published is None:
        return True
    status = fetch(f'repos/{repo}/compare/{published}...{sha}')['status']
    if status == 'ahead':
        return True
    if status in ('identical', 'behind'):
        return False
    raise ValueError('Published revision and candidate diverge; refusing publication')


def mac_feed_revision(repo, previous, fetch=api):
    if not previous.strip():
        return None
    root = ET.fromstring(previous)
    enclosure = root.find('./channel/item/enclosure')
    if enclosure is None:
        raise ValueError('Previous Mac feed has no release identity')
    match = re.fullmatch(r'https://github.com/' + re.escape(repo) +
                        r'/releases/download/(preview-mac-[0-9]+-[0-9]+)/[^/]+',
                        enclosure.get('url', ''))
    if not match:
        raise ValueError('Unknown previous Mac release URL')
    target = fetch(f'repos/{repo}/git/ref/tags/{match[1]}')['object']
    if target['type'] != 'commit':
        raise ValueError('Previous Mac release tag must identify a commit')
    # Published preview predates the signing-history rewrite. This single
    # reviewed identity may use its content-identical canonical counterpart.
    if repo == 'tryapparent/apparent' and match[1] == 'preview-mac-12-1':
        original = '75dc870c240e415820384ced32f4390f798615bc'
        canonical = '65cee9e56edca6fe629a93ff9aafcbf35cb8608e'
        tree = '27dc97069363fa1b6b838a421adc4bbcd6f9c38f'
        if target['sha'] != original:
            raise ValueError('Historical Mac release tag changed')
        for revision in (original, canonical):
            commit = fetch(f'repos/{repo}/git/commits/{revision}')
            if commit['sha'] != revision or commit['tree']['sha'] != tree:
                raise ValueError('Historical Mac source mapping is not content-identical')
        return canonical
    return target['sha']


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', choices=sorted(WORKFLOWS), required=True)
    parser.add_argument('--sha', required=True)
    parser.add_argument('--run-id')
    parser.add_argument('--previous-mac-feed', type=Path)
    args = parser.parse_args()
    sha = admit(os.environ['GITHUB_REPOSITORY'], args.sha, args.run_id, platform=args.platform)
    if args.previous_mac_feed is not None:
        if args.platform != 'macos':
            raise ValueError('Mac feed ordering requires macos admission')
        published = mac_feed_revision(os.environ['GITHUB_REPOSITORY'], args.previous_mac_feed.read_text())
        if not publication_needed(os.environ['GITHUB_REPOSITORY'], sha, published):
            raise ValueError('Mac feed already contains this or a newer source revision')
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        output.write(f'sha={sha}\n')
    print(f'Admitted verified main commit {sha}')


if __name__ == '__main__':
    main()
