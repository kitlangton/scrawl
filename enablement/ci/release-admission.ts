/** Main CD evidence policy. Manual receipts remain in the legacy admission adapter. */
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

type Job = { name: string; status: string; conclusion: string | null };
type Run = {
  repository: { full_name: string }; head_repository: { full_name: string };
  path: string; head_sha: string; head_branch: string; event: string;
  status: string; conclusion: string | null;
};
export function admitsCD(run: Run, jobs: Job[], repo: string, sha: string, platform: string): boolean {
  if (!['linux', 'macos'].includes(platform)) return false;
  if (run.repository.full_name !== repo || run.head_repository.full_name !== repo ||
      run.path !== '.github/workflows/checks.yml' || run.head_sha !== sha ||
      run.head_branch !== 'main' || run.event !== 'push') return false;
  if (!((run.status === 'completed' && ['success', 'failure'].includes(run.conclusion ?? '')) ||
        (run.status === 'in_progress' && run.conclusion === null))) return false;
  // A failed Mac job must not prevent Linux publication. Require this platform's
  // actual jobs, including shared admission guards, rather than aggregate success.
  const runner = platform === 'linux' ? 'ubuntu-24.04' : 'macos-15-intel';
  const required = ['contracts', 'select', 'Hermes inspection (ubuntu-24.04)',
    `${platform} / select`, ...['desktop-ui', 'connect-client', 'rust'].map(name => `${platform} / ${name} (${runner})`)];
  if (platform === 'macos') required.push('Hermes inspection (macos-15-intel)');
  const passed = new Set(jobs.filter(job => job.status === 'completed' && job.conclusion === 'success').map(job => job.name));
  return required.every(name => passed.has(name));
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const { run, jobs, repo, sha, platform } = JSON.parse(readFileSync(0, 'utf8'));
  process.stdout.write(JSON.stringify(admitsCD(run, jobs, repo, sha, platform)));
}
