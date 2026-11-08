import {execFileSync} from 'node:child_process';
import {readFileSync} from 'node:fs';
import {pathToFileURL} from 'node:url';
export const repo = 'tryapparent/apparent';
const prefix = `repos/${repo}`;
export type API = (path: string, body?: unknown) => any;
export const api: API = (path, body) => JSON.parse(execFileSync('gh', ['api', path, ...(body ? ['--method', 'POST', '--input', '-'] : [])], {input: body ? JSON.stringify(body) : undefined, encoding: 'utf8'}));
export function pages(fetch: API, path: string, key?: string): any[] {
  const all: any[] = [];
  for (let page = 1; page <= 1000; page++) {
    const data = fetch(`${path}${path.includes('?') ? '&' : '?'}per_page=100&page=${page}`);
    const rows = key ? data[key] : data;
    if (!Array.isArray(rows)) throw Error('Invalid inventory');
    all.push(...rows);
    if (key ? all.length >= data.total_count : rows.length < 100) return all;
    if (!rows.length) break;
  }
  throw Error('Incomplete inventory');
}
export function scope(paths: string[]): 'docs' | 'public' | 'native' {
  // Only owner documentation is exempt; unknown paths and shared inputs are broad.
  const docs = (p: string) => /^(capture\/(engine|desktop|agent)\/docs\/|enablement\/docs\/|public\/site\/docs\/|connect\/.*\/docs\/).+\.md$/.test(p);
  if (paths.length && paths.every(docs)) return 'docs';
  if (paths.length && paths.every(p => docs(p) || p.startsWith('public/site/'))) return 'public';
  return 'native';
}
export function inventory(source: string, runner: string): string[] {
  const ids = [...(source.split('\njobs:\n')[1] ?? source).matchAll(/^  ([\w-]+):\s*$/gm)].map(m => m[1]);
  const current = ['select', 'desktop-ui', 'rust', 'release-ready'];
  const parallel = [...current, 'connect-client', 'connect-browser'];
  if (![current, parallel].some(set => set.length === ids.length && set.every(id => ids.includes(id)))) throw Error('Unknown platform job inventory');
  return ids.filter(id => id !== 'release-ready').map(id => id === 'select' ? 'verify / select' : `verify / ${id} (${runner})`);
}
export function evaluate(run: any, jobs: any[], identity: any, required: string[]): string {
  if (run.repository?.full_name !== repo || run.head_repository?.full_name !== repo || run.path !== identity.path || run.head_sha !== identity.sha || run.head_branch !== identity.branch || run.event !== 'workflow_dispatch' || !Number.isInteger(run.run_attempt) || run.run_attempt < 1) return 'failure';
  if (run.status !== 'completed') return 'pending';
  if (run.conclusion !== 'success') return 'failure';
  for (const name of required) {
    const matches = jobs.filter(j => j.name === name);
    if (matches.length !== 1 || matches[0].status !== 'completed' || matches[0].conclusion !== 'success' || matches[0].run_id !== run.id) return 'failure';
    // A green job must not conceal a skipped required native assertion.
    for (const step of matches[0].steps ?? []) {
      if (/Native Mac Storage|Native GPUI Copy|Native Electron selected-reference/.test(step.name) && step.conclusion !== 'success') return 'failure';
    }
  }
  if (identity.path.includes('verification') && required.some(n => n.includes(' / rust '))) {
    const steps = jobs.flatMap(j => j.steps ?? []);
    for (const pattern of [/Execute the same changed-tree checks/, /(?:Actual Electron to Rust Settings integration|Early Mac Electron to Rust Settings integration)/]) {
      if (!steps.some(s => pattern.test(s.name) && s.conclusion === 'success')) return 'failure';
    }
  }
  if (identity.path.includes('mac')) {
    const steps = jobs.flatMap(j => j.steps ?? []);
    for (const name of ['Native Mac Storage', 'Native GPUI Copy', 'Native Electron selected-reference']) {
      if (!steps.some(s => s.name.startsWith(name) && s.conclusion === 'success')) return 'failure';
    }
  }
  return 'success';
}
function content(fetch: API, path: string, ref: string): string {
  return Buffer.from(fetch(`${prefix}/contents/${path}?ref=${encodeURIComponent(ref)}`).content, 'base64').toString();
}
export function reconcile(fetch: API, number: number): void {
  const read = () => fetch(`${prefix}/pulls/${number}`);
  const candidate = read();
  if (candidate.state !== 'open' || candidate.base.ref !== 'main' || candidate.base.repo.full_name !== repo) return;
  const sha = candidate.head.sha;
  const fingerprint = (p: any) => JSON.stringify([p.state, p.head.sha, p.head.ref, p.head.repo?.full_name, p.base.sha, p.base.ref]);
  let state = 'pending';
  let description = 'Manual exact-candidate verification required';
  const evidence: any[] = [];
  try {
    if (candidate.head.repo?.full_name !== repo) throw Error('Fork requires trusted verification');
    const comparison = fetch(`${prefix}/compare/${candidate.base.sha}...${sha}`);
    if (comparison.files?.length >= 300 || !comparison.files?.length) throw Error('Incomplete or empty delta');
    const kind = scope(comparison.files.flatMap((f: any) => [f.filename, ...(f.previous_filename ? [f.previous_filename] : [])]));
    // New relevant base inputs invalidate proof; unrelated documentation does not.
    if (comparison.merge_base_commit.sha !== candidate.base.sha) {
      const baseDelta = fetch(`${prefix}/compare/${comparison.merge_base_commit.sha}...${candidate.base.sha}`);
      if (!baseDelta.files?.length || baseDelta.files.length >= 300 || scope(baseDelta.files.map((f: any) => f.filename)) !== 'docs') throw Error('Integrate changed base inputs');
    }
    if (kind === 'public') {
      const path = '.github/workflows/stream-public.yml';
      if (content(fetch, path, sha) !== content(fetch, path, candidate.base.sha)) throw Error('Public policy differs from trusted base');
      const runs = pages(fetch, `${prefix}/actions/workflows/stream-public.yml/runs?head_sha=${sha}&branch=${encodeURIComponent(candidate.head.ref)}`, 'workflow_runs');
      const latest = runs.sort((a, b) => b.id - a.id)[0];
      if (!latest) throw Error('Dispatch Public checks on exact head for Public-only proof');
      const run = fetch(`${prefix}/actions/runs/${latest.id}`);
      evidence.push(run);
      const jobs = pages(fetch, `${prefix}/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs`, 'jobs');
      state = evaluate(run, jobs, {path, sha, branch: candidate.head.ref}, ['verify']);
    }
    if (kind === 'docs') state = 'success';
    if (kind === 'native') {
      const source = content(fetch, '.github/workflows/verification-platform.yml', candidate.base.sha);
      if (content(fetch, '.github/workflows/verification-platform.yml', sha) !== source) throw Error('Platform policy differs from trusted base');
      const results = ['verification.yml', 'verification-mac.yml'].map((filename, index) => {
        const path = `.github/workflows/${filename}`;
        if (content(fetch, path, sha) !== content(fetch, path, candidate.base.sha)) throw Error('Caller differs from trusted base');
        const runs = pages(fetch, `${prefix}/actions/workflows/${filename}/runs?head_sha=${sha}&branch=${encodeURIComponent(candidate.head.ref)}`, 'workflow_runs');
        const latest = runs.sort((a, b) => b.id - a.id)[0];
        if (!latest) return 'pending';
        const run = fetch(`${prefix}/actions/runs/${latest.id}`);
        const jobs = pages(fetch, `${prefix}/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs`, 'jobs');
        evidence.push(run);
        return evaluate(run, jobs, {path, sha, branch: candidate.head.ref}, inventory(source, index ? 'macos-15-intel' : 'ubuntu-24.04'));
      });
      state = results.includes('failure') ? 'failure' : results.includes('pending') ? 'pending' : 'success';
    }
    description = state === 'success' ? 'Applicable verification complete; not release admission' : state === 'failure' ? 'Required verification failed or skipped' : description;
  } catch (error) { description = String((error as Error).message).slice(0, 140); }
  // Serialized workflow writers re-read both candidate and evidence. Never publish
  // a terminal answer from an obsolete attempt or an obsolete PR candidate.
  if (fingerprint(read()) !== fingerprint(candidate)) return;
  for (const run of evidence) {
    const filename = run.path.split('/').at(-1);
    const latest = pages(fetch, `${prefix}/actions/workflows/${filename}/runs?head_sha=${sha}&branch=${encodeURIComponent(candidate.head.ref)}`, 'workflow_runs').sort((a, b) => b.id - a.id)[0];
    const fresh = fetch(`${prefix}/actions/runs/${run.id}`);
    if (latest?.id !== run.id) {state = 'pending'; description = 'Newer verification requires reconciliation';}
    if (fresh.run_attempt !== run.run_attempt || fresh.status !== run.status || fresh.conclusion !== run.conclusion) {state = 'pending'; description = 'Verification changed during reconciliation';}
  }
  if (fingerprint(read()) !== fingerprint(candidate)) return;
  fetch(`${prefix}/statuses/${sha}`, {state, context: 'Verification complete', description, target_url: candidate.html_url});
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH!, 'utf8')); // Refuse malformed event payloads.
  if (process.env.GITHUB_REPOSITORY !== repo) throw Error('Unexpected repository');
  const prs = pages(api, `${prefix}/pulls?state=open&base=main`);
  for (const pr of prs) reconcile(api, pr.number);
}
