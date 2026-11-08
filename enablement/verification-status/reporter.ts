import {execFileSync} from 'node:child_process';
import {appendFileSync, readFileSync} from 'node:fs';
import {pathToFileURL} from 'node:url';
import {YAML} from 'bun';
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
// This is completion admission, not a sandbox or a source-code security review.
// Workflow bodies, like the tests they invoke, still require independent review.
export function policy(source: string, runner: string): {names: string[], steps: Record<string, string[]>} {
  const names = inventory(source, runner);
  const document = YAML.parse(source);
  const steps: Record<string, string[]> = {};
  for (const name of names) {
    const id = name === 'verify / select' ? 'select' : name.split(' / ')[1].split(' ')[0];
    const job = document.jobs[id];
    if (job['continue-on-error'] || !Array.isArray(job.steps)) throw Error('Unsupported required job');
    steps[name] = job.steps.filter((s: any) => s.run && !s.if?.includes('always()') && !(runner.startsWith('ubuntu') ? s.if?.includes("runner.os == 'macOS'") : s.if?.includes("runner.os == 'Linux'"))).map((s: any) => {
      if (s['continue-on-error']) throw Error('Required step tolerates failure');
      return s.name || (s.uses ? `Run ${s.uses}` : `Run ${s.run.split('\n')[0]}`);
    });
  }
  const rust = document.jobs.rust.steps;
  for (const command of ['enablement/verification.py', '_electron_rust_roundtrip']) {
    if (!rust.some((s: any) => s.run?.includes(command))) throw Error(`Missing required command ${command}`);
  }
  for (const prefix of ['Native Mac Storage', 'Native GPUI Copy', 'Native Electron selected-reference']) {
    if (!rust.some((s: any) => s.name?.startsWith(prefix) && s.run)) throw Error(`Missing ${prefix}`);
  }
  return {names, steps};
}
export function publicPolicy(source: string): Record<string, string[]> {
  const job = YAML.parse(source).jobs?.verify;
  if (!job || job['continue-on-error'] || !Array.isArray(job.steps) || !job.steps.some((s: any) => s.run?.includes('npm run verify'))) throw Error('Unsupported Public verification policy');
  const steps = job.steps.filter((s: any) => s.run && !s.if?.includes('always()'));
  if (steps.some((s: any) => s['continue-on-error'])) throw Error('Public verification tolerates failure');
  return {verify: steps.map((s: any) => s.name || `Run ${s.run.split('\n')[0]}`)};
}
export function evaluate(run: any, jobs: any[], identity: any, required: string[], expected: Record<string, string[]> = {}): string {
  if (run.repository?.full_name !== repo || run.head_repository?.full_name !== repo || run.path !== identity.path || run.head_sha !== identity.sha || run.head_branch !== identity.branch || run.event !== 'workflow_dispatch' || !Number.isInteger(run.run_attempt) || run.run_attempt < 1) return 'failure';
  if (run.status !== 'completed') return 'pending';
  if (run.conclusion !== 'success') return 'failure';
  for (const name of required) {
    const matches = jobs.filter(j => j.name === name);
    if (matches.length !== 1 || matches[0].status !== 'completed' || matches[0].conclusion !== 'success' || matches[0].run_id !== run.id) return 'failure';
    for (const stepName of expected[name] ?? []) {
      const assertions = (matches[0].steps ?? []).filter((s: any) => s.name === stepName);
      if (assertions.length !== 1 || assertions[0].conclusion !== 'success') return 'failure';
    }
    const expectedOrder = (expected[name] ?? []).map(stepName => (matches[0].steps ?? []).findIndex((s: any) => s.name === stepName));
    if (expectedOrder.some((position, i) => i > 0 && position <= expectedOrder[i - 1])) return 'failure';
    // A green job must not conceal a skipped required native assertion.
    for (const step of matches[0].steps ?? []) {
      if (identity.path.includes('mac') && /Native Mac Storage|Native GPUI Copy|Native Electron selected-reference/.test(step.name) && step.conclusion !== 'success') return 'failure';
    }
  }
  if (identity.path.includes('verification') && required.some(n => n.includes(' / rust '))) {
    const steps = jobs.filter(j => /verify \/ rust \(/.test(j.name)).flatMap(j => j.steps ?? []);
    for (const pattern of [/Execute the same changed-tree checks/, /(?:Actual Electron to Rust Settings integration|Early Mac Electron to Rust Settings integration)/]) {
      if (!steps.some(s => pattern.test(s.name) && s.conclusion === 'success')) return 'failure';
    }
  }
  if (identity.path.includes('mac')) {
    const steps = jobs.filter(j => /verify \/ rust(?: \(|$)/.test(j.name)).flatMap(j => j.steps ?? []);
    for (const name of ['Native Mac Storage', 'Native GPUI Copy', 'Native Electron selected-reference']) {
      if (!steps.some(s => s.name.startsWith(name) && s.conclusion === 'success')) return 'failure';
    }
  }
  return 'success';
}
function content(fetch: API, path: string, ref: string): string {
  return Buffer.from(fetch(`${prefix}/contents/${path}?ref=${encodeURIComponent(ref)}`).content, 'base64').toString();
}
// GitHub only permits repository writers to dispatch these manual workflows.
// Revalidate that authority; completion status does not replace source review.
export function dispatchAuthority(fetch: API, run: any): string {
  const login = run.triggering_actor?.login;
  if (run.event !== 'workflow_dispatch' || typeof login !== 'string' || !login) throw Error('Missing manual verification actor');
  const permission = fetch(`${prefix}/collaborators/${encodeURIComponent(login)}/permission`).permission;
  if (!['admin', 'maintain', 'write'].includes(permission)) throw Error('Manual verification actor lacks repository write access');
  return JSON.stringify([login, permission]);
}
export function reconcile(fetch: API, number: number, listed?: any): void {
  // Invalidate known old green before any per-candidate evidence/API reads.
  const pending = (p: any) => fetch(`${prefix}/statuses/${p.head.sha}`, {state: 'pending', context: 'Verification complete', description: 'Rechecking current verification evidence', target_url: p.html_url});
  if (listed) pending(listed);
  const read = () => fetch(`${prefix}/pulls/${number}`);
  const candidate = read();
  if (candidate.state !== 'open' || candidate.base.ref !== 'main' || candidate.base.repo.full_name !== repo) return;
  const sha = candidate.head.sha;
  if (!listed || listed.head.sha !== sha) pending(candidate);
  const fingerprint = (p: any) => JSON.stringify([p.state, p.head.sha, p.head.ref, p.head.repo?.full_name, p.base.sha, p.base.ref]);
  let state = 'pending';
  let description = 'Manual exact-candidate verification required';
  const evidence: any[] = [];
  let baseSha: string | undefined;
  const readBase = () => {
    const sha = fetch(`${prefix}/git/ref/heads/main`).object?.sha;
    if (typeof sha !== 'string' || !sha) throw Error('Missing live main ref');
    return sha;
  };
  try {
    baseSha = readBase();
    if (candidate.head.repo?.full_name !== repo) throw Error('Fork requires trusted verification');
    const comparison = fetch(`${prefix}/compare/${baseSha}...${sha}`);
    if (comparison.files?.length >= 300 || !comparison.files?.length) throw Error('Incomplete or empty delta');
    const kind = scope(comparison.files.flatMap((f: any) => [f.filename, ...(f.previous_filename ? [f.previous_filename] : [])]));
    // New relevant base inputs invalidate proof; unrelated documentation does not.
    if (comparison.merge_base_commit.sha !== baseSha) {
      const baseDelta = fetch(`${prefix}/compare/${comparison.merge_base_commit.sha}...${baseSha}`);
      if (!baseDelta.files?.length || baseDelta.files.length >= 300 || scope(baseDelta.files.flatMap((f: any) => [f.filename, ...(f.previous_filename ? [f.previous_filename] : [])])) !== 'docs') throw Error('Integrate changed base inputs');
    }
    if (kind === 'public') {
      const path = '.github/workflows/stream-public.yml';
      const expected = publicPolicy(content(fetch, path, sha));
      const runs = pages(fetch, `${prefix}/actions/workflows/stream-public.yml/runs?head_sha=${sha}&branch=${encodeURIComponent(candidate.head.ref)}`, 'workflow_runs');
      const latest = runs.sort((a, b) => b.id - a.id)[0];
      if (!latest) throw Error('Dispatch Public checks on exact head for Public-only proof');
      const run = fetch(`${prefix}/actions/runs/${latest.id}`);
      evidence.push({...run, authority: dispatchAuthority(fetch, run)});
      const jobs = pages(fetch, `${prefix}/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs`, 'jobs');
      state = evaluate(run, jobs, {path, sha, branch: candidate.head.ref}, ['verify'], expected);
    }
    if (kind === 'docs') state = 'success';
    if (kind === 'native') {
      const source = content(fetch, '.github/workflows/verification-platform.yml', sha);
      const results = ['verification.yml', 'verification-mac.yml'].map((filename, index) => {
        const path = `.github/workflows/${filename}`;
        const caller = YAML.parse(content(fetch, path, sha));
        const runner = index ? 'macos-15-intel' : 'ubuntu-24.04';
        if (caller.jobs?.verify?.uses !== './.github/workflows/verification-platform.yml' || caller.jobs.verify.with?.runner !== runner || caller.jobs.verify.if || caller.jobs.verify['continue-on-error']) throw Error('Unsupported verification caller');
        const admitted = policy(source, runner);
        const trusted = policy(content(fetch, '.github/workflows/verification-platform.yml', baseSha), runner);
        admitted.names = [...new Set([...trusted.names, ...admitted.names])];
        if (admitted.names.some(name => !Object.hasOwn(admitted.steps, name))) throw Error('Candidate removed a required job');
        const runs = pages(fetch, `${prefix}/actions/workflows/${filename}/runs?head_sha=${sha}&branch=${encodeURIComponent(candidate.head.ref)}`, 'workflow_runs');
        const latest = runs.sort((a, b) => b.id - a.id)[0];
        if (!latest) return 'pending';
        const run = fetch(`${prefix}/actions/runs/${latest.id}`);
        const jobs = pages(fetch, `${prefix}/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs`, 'jobs');
        evidence.push({...run, authority: dispatchAuthority(fetch, run)});
        return evaluate(run, jobs, {path, sha, branch: candidate.head.ref}, admitted.names, admitted.steps);
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
    try {
      if (dispatchAuthority(fetch, fresh) !== run.authority) {state = 'pending'; description = 'Manual verification authority changed';}
    } catch {state = 'pending'; description = 'Manual verification authority no longer valid';}
    if (latest?.id !== run.id) {state = 'pending'; description = 'Newer verification requires reconciliation';}
    if (fresh.run_attempt !== run.run_attempt || fresh.status !== run.status || fresh.conclusion !== run.conclusion) {state = 'pending'; description = 'Verification changed during reconciliation';}
  }
  if (fingerprint(read()) !== fingerprint(candidate)) return;
  // PR base.sha can remain pinned to an old main commit. Authenticate live main
  // again after all evidence reads instead of trusting that cached PR field.
  if (baseSha && readBase() !== baseSha) {state = 'pending'; description = 'Main advanced during reconciliation';}
  const report = process.env.GITHUB_RUN_ID ? `https://github.com/${repo}/actions/runs/${process.env.GITHUB_RUN_ID}` : candidate.html_url;
  if (process.env.GITHUB_STEP_SUMMARY) {
    appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### PR #${number}: ${state}\n\nCandidate: \`${sha}\`\n\n${evidence.map(run => `- [${run.path.includes('mac') ? 'Mac' : run.path.includes('public') ? 'Public' : 'Linux'} verification](https://github.com/${repo}/actions/runs/${run.id}) — ${run.status} / ${run.conclusion ?? 'pending'}`).join('\n')}\n\n`);
  }
  fetch(`${prefix}/statuses/${sha}`, {state, context: 'Verification complete', description, target_url: report});
}
export function reconcileAll(fetch: API, event: any = {}): void {
  // A lifecycle event supplies a known target even if listing all PRs fails.
  const hint = event.pull_request;
  if (hint?.head?.sha && hint.base?.repo?.full_name === repo && hint.base.ref === 'main') {
    fetch(`${prefix}/statuses/${hint.head.sha}`, {state: 'pending', context: 'Verification complete', description: 'Rechecking current verification evidence', target_url: hint.html_url});
  }
  const manual = event.workflow_run;
  if (manual?.head_repository?.full_name === repo && manual.pull_requests?.some((p: any) => p.head?.sha === manual.head_sha && p.base?.ref === 'main')) {
    fetch(`${prefix}/statuses/${manual.head_sha}`, {state: 'pending', context: 'Verification complete', description: 'Rechecking manual verification event', target_url: `https://github.com/${repo}/actions/runs/${manual.id}`});
  }
  const prs = pages(fetch, `${prefix}/pulls?state=open&base=main`);
  let failed = false;
  for (const pr of prs) {
    try { reconcile(fetch, pr.number, pr); }
    catch { failed = true; console.error(`Verification reconciliation incomplete for PR #${pr.number}`); }
  }
  if (failed) throw Error('One or more PR reconciliations failed; known heads remain pending');
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH!, 'utf8'));
  if (process.env.GITHUB_REPOSITORY !== repo) throw Error('Unexpected repository');
  reconcileAll(api, event);
}
