#!/usr/bin/env python3
"""Admit exact verified publication commits; main is the default policy.

The approved onboarding Preview exception is repository/branch/platform bounded.
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


def verified_run(repo, sha, platform, run_id, fetch, *, onboarding_candidate=False):
    prefix = f'repos/{repo}'
    workflow = WORKFLOWS[platform]
    branch = 't3/onboarding-mac-release-baseline' if onboarding_candidate else 'main'
    events = {'workflow_dispatch'} if onboarding_candidate else {'push', 'workflow_dispatch'}
    if run_id:
        runs = [fetch(f'{prefix}/actions/runs/{run_id}')]
    else:
        filename = workflow.rsplit('/', 1)[1]
        runs = pages(f'{prefix}/actions/workflows/{filename}/runs?head_sha={sha}&branch={branch}', fetch)
    # Never accept another commit, fork, workflow, branch/event or old attempt.
    for candidate in sorted(runs, key=lambda run: run['id'], reverse=True):
        run = fetch(f'{prefix}/actions/runs/{candidate["id"]}')
        if not (run['repository']['full_name'] == repo
                and run['head_repository']['full_name'] == repo
                and run['path'] == workflow and run['head_sha'] == sha
                and run['head_branch'] == branch and run['event'] in events
                and run['status'] == 'completed' and run['conclusion'] == 'success'):
            continue
        jobs = pages(f'{prefix}/actions/runs/{run["id"]}/attempts/{run["run_attempt"]}/jobs', fetch)
        successful = {job['name'] for job in jobs
                      if job['status'] == 'completed' and job['conclusion'] == 'success'}
        required = REQUIRED[platform] | ({'onboarding-candidate'} if onboarding_candidate and platform == 'macos' else set())
        if required <= successful:
            return
    raise ValueError(f'No successful {platform} {branch} verification for this exact commit')


def admit(repo, sha, run_id=None, fetch=api, platform='linux', *, onboarding_candidate=False):
    if platform not in WORKFLOWS:
        raise ValueError('Expected linux or macos publication platform')
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repo):
        raise ValueError('Invalid repository')
    if not re.fullmatch(r'[0-9a-f]{40}', sha):
        raise ValueError('Expected an exact commit SHA')
    if run_id is not None and not str(run_id).isdigit():
        raise ValueError('Invalid verification run ID')
    if onboarding_candidate and (repo != 'tryapparent/apparent' or platform != 'macos'):
        raise ValueError('Onboarding exception is only for this repository Mac Preview')
    # Linux also covers shared/browser contracts. Mac publication retains that
    # receipt plus its own native platform checks. No Mac lookup blocks Linux.
    platforms = ['linux'] if platform == 'linux' else ['linux', 'macos']
    for required in platforms:
        verified_run(repo, sha, required, run_id if required == platform else None, fetch, onboarding_candidate=onboarding_candidate)
    if onboarding_candidate:
        baseline = '75dc870c240e415820384ced32f4390f798615bc'
        status = fetch(f'repos/{repo}/compare/{baseline}...{sha}')['status']
        if status != 'ahead':
            raise ValueError('Candidate must descend from published Preview 0.12.1')
        live = fetch(f'repos/{repo}/git/ref/heads/t3/onboarding-mac-release-baseline')
        if live['object']['type'] != 'commit' or live['object']['sha'] != sha:
            raise ValueError('Candidate branch moved after verification')
        return sha
    status = fetch(f'repos/{repo}/compare/{sha}...main')['status']
    if status not in ('ahead', 'identical'):
        raise ValueError('Verified commit is not on main')
    return sha


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', choices=sorted(WORKFLOWS), required=True)
    parser.add_argument('--sha', required=True)
    parser.add_argument('--run-id')
    parser.add_argument('--onboarding-candidate', action='store_true')
    args = parser.parse_args()
    sha = admit(os.environ['GITHUB_REPOSITORY'], args.sha, args.run_id, platform=args.platform, onboarding_candidate=args.onboarding_candidate)
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        output.write(f'sha={sha}\n')
    print(f'Admitted verified commit {sha}')


if __name__ == '__main__':
    main()
