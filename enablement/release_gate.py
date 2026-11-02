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

WORKFLOWS = {
    'linux': '.github/workflows/verification.yml',
    'macos': '.github/workflows/verification-mac.yml',
}
REQUIRED = {
    platform: {'verify / select', f'verify / desktop-ui ({runner})',
               f'verify / rust ({runner})', 'verify / Release verification'}
    for platform, runner in [('linux', 'ubuntu-24.04'), ('macos', 'macos-latest')]
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
        runs = pages(f'{prefix}/actions/workflows/{filename}/runs?head_sha={sha}&branch=main&event=push', fetch)
    # Never accept a different commit, fork, workflow, PR/manual run or old attempt.
    for candidate in sorted(runs, key=lambda run: run['id'], reverse=True):
        run = fetch(f'{prefix}/actions/runs/{candidate["id"]}')
        if not (run['repository']['full_name'] == repo
                and run['head_repository']['full_name'] == repo
                and run['path'] == workflow and run['head_sha'] == sha
                and run['head_branch'] == 'main' and run['event'] == 'push'
                and run['status'] == 'completed' and run['conclusion'] == 'success'):
            continue
        jobs = pages(f'{prefix}/actions/runs/{run["id"]}/attempts/{run["run_attempt"]}/jobs', fetch)
        successful = {job['name'] for job in jobs
                      if job['status'] == 'completed' and job['conclusion'] == 'success'}
        if REQUIRED[platform] <= successful:
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', choices=sorted(WORKFLOWS), required=True)
    parser.add_argument('--sha', required=True)
    parser.add_argument('--run-id')
    args = parser.parse_args()
    sha = admit(os.environ['GITHUB_REPOSITORY'], args.sha, args.run_id, platform=args.platform)
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        output.write(f'sha={sha}\n')
    print(f'Admitted verified main commit {sha}')


if __name__ == '__main__':
    main()
