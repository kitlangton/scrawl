import { spawn } from 'node:child_process';
import { constants } from 'node:fs';
import { open } from 'node:fs/promises';

const REPOSITORY = 'tryapparent/apparent';
const MAX_BODY = 60_000;
const MAX_STDOUT = 512 * 1024;

export class IssueCliError extends Error {
  constructor(message: string, readonly exitCode: 1 | 2) { super(message); }
}

function usage(message: string): never { throw new IssueCliError(`${message}\nRun 'apparent-agent issue --help' for usage.`, 2); }
function execution(message: string): never { throw new IssueCliError(message, 1); }

export function issueHelp(): string {
  return `apparent-agent issue — prepare text-only GitHub feedback\n\nUsage: apparent-agent issue --title <TEXT> (--body <TEXT> | --body-file <PATH>) [--submit]\n\nOptions:\n  --title TEXT       Required issue title (1–256 Unicode scalar values)\n  --body TEXT        Exact Markdown body\n  --body-file PATH   Exact UTF-8 body from a regular file (maximum 60000 bytes)\n  --submit           Publish publicly to ${REPOSITORY} using gh on github.com\n  -h, --help         Show this help without invoking gh\n\nWithout --submit, stdout is a JSON preview and no network or login is needed.\nBefore submitting, search existing issues and obtain the human's approval of the\nexact repository, title and body. No captures, attachments, config or diagnostics\nare collected. Submission is noninteractive and verified by reading the issue back.\nIf creation or readback fails, inspect the repository before retrying.\nSee capture/desktop/docs/issue-reporting.md.\n`;
}

type IssueOptions = { title: string; body?: string; bodyFile?: string; submit: boolean };

function parse(args: string[]): IssueOptions | 'help' {
  let title: string | undefined, body: string | undefined, bodyFile: string | undefined;
  let submit = false, seenSubmit = false;
  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === '--help' || arg === '-h') return 'help';
    if (arg === '--submit') {
      if (seenSubmit) usage('Duplicate --submit.');
      seenSubmit = true; submit = true; continue;
    }
    const match = /^(--title|--body|--body-file)=(.*)$/s.exec(arg);
    const option = match?.[1] ?? arg;
    if (!['--title', '--body', '--body-file'].includes(option)) usage(`Unknown option: ${arg}`);
    const value = match ? match[2] : args[++i];
    if (value === undefined) usage(`${option} requires a value.`);
    if (!match && (value.startsWith('-') || value === '-h')) usage(`${option} requires a value; use ${option}=TEXT when the value begins with a dash.`);
    if (option === '--title') {
      if (title !== undefined) usage('Duplicate --title.');
      title = value;
    } else if (option === '--body') {
      if (body !== undefined) usage('Duplicate --body.');
      body = value;
    } else {
      if (bodyFile !== undefined) usage('Duplicate --body-file.');
      bodyFile = value;
    }
  }
  if (title === undefined) usage('Missing required --title.');
  if ((body === undefined) === (bodyFile === undefined)) usage('Provide exactly one of --body or --body-file.');
  return { title, body, bodyFile, submit };
}

async function readBody(path: string): Promise<string> {
  let file;
  try { file = await open(path, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOFOLLOW); }
  catch { execution('Could not open --body-file as a regular UTF-8 text file (nothing sent).'); }
  try {
    try {
      const info = await file!.stat();
      if (!info.isFile()) execution('--body-file must be a regular file (nothing sent).');
      if (info.size > MAX_BODY) execution('--body-file exceeds 60000 UTF-8 bytes (nothing sent).');
      const bytes = Buffer.alloc(MAX_BODY + 1);
      let bytesRead = 0;
      while (bytesRead < bytes.length) {
        const read = await file!.read(bytes, bytesRead, bytes.length - bytesRead, bytesRead);
        if (read.bytesRead === 0) break;
        bytesRead += read.bytesRead;
      }
      if (bytesRead > MAX_BODY) execution('--body-file exceeds 60000 UTF-8 bytes (nothing sent).');
      try { return new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes.subarray(0, bytesRead)); }
      catch { execution('--body-file must contain valid UTF-8 (nothing sent).'); }
    } catch (error) {
      if (error instanceof IssueCliError) throw error;
      execution('Could not read --body-file as a regular UTF-8 text file (nothing sent).');
    }
  } finally { await file!.close().catch(() => {}); }
}

function validate(title: string, body: string): void {
  const scalars = [...title].length;
  if (!title.trim() || scalars > 256 || [...title].some(char => /\p{Cc}/u.test(char)))
    usage('Title must be nonempty, at most 256 Unicode scalar values, and contain no control characters.');
  if (!body.trim() || Buffer.byteLength(body, 'utf8') > MAX_BODY || body.includes('\0'))
    usage('Body must be nonempty UTF-8 text, at most 60000 bytes, and contain no NUL.');
}

type ChildResult = { code: number | null; signal: NodeJS.Signals | null; stdout: Buffer };

/** Spawn one owned command without a shell. Kept parameterized for fast transport-bound tests. */
export async function runGh(args: string[], timeoutMs = 30_000): Promise<ChildResult> {
  const deadlineAt = Date.now() + timeoutMs;
  const child = spawn('gh', args, {
    shell: false, detached: true, windowsHide: true,
    env: { ...process.env, GH_HOST: 'github.com', GH_PROMPT_DISABLED: '1', GH_PAGER: 'cat' },
    stdio: ['ignore', 'pipe', 'ignore'],
  });
  const chunks: Buffer[] = []; let total = 0, done = false, timer: NodeJS.Timeout;
  let resolveClose!: (result: ChildResult) => void;
  let rejectClose!: (error: Error) => void;
  const closed = new Promise<ChildResult>((resolve, reject) => { resolveClose = resolve; rejectClose = reject; });
  const terminate = () => {
    if (child.pid !== undefined) {
      try { process.kill(-child.pid, 'SIGKILL'); } catch { /* process group may have exited */ }
      try { child.kill('SIGKILL'); } catch { /* leader may have exited */ }
    }
    child.stdout?.destroy();
  };
  const fail = (message: string) => {
    if (done) return;
    done = true; terminate();
    // Reap the owned leader when possible, but never let a broken descendant
    // hold cleanup past this short grace period.
    void new Promise<void>(resolve => {
      const grace = setTimeout(resolve, 1_000);
      child.once('close', () => { clearTimeout(grace); resolve(); });
    }).then(() => rejectClose(new Error(message)));
  };
  timer = setTimeout(() => fail('GitHub CLI exceeded its 30-second deadline; inspect the repository before retrying because an issue may already exist.'), timeoutMs);
  child.stdout?.on('data', (chunk: Buffer) => {
    total += chunk.length;
    if (total > MAX_STDOUT) { fail('GitHub CLI output exceeded 512 KiB; inspect the repository before retrying because an issue may already exist.'); return; }
    chunks.push(Buffer.from(chunk));
  });
  child.once('error', () => fail('Could not start GitHub CLI. Install gh and authenticate with `gh auth login --hostname github.com`.'));
  child.once('close', (code, signal) => {
    if (done) return;
    if (Date.now() >= deadlineAt) { fail('GitHub CLI exceeded its 30-second deadline; inspect the repository before retrying because an issue may already exist.'); return; }
    done = true; resolveClose({ code, signal, stdout: Buffer.concat(chunks, total) });
  });
  try { return await closed; }
  finally { clearTimeout(timer!); }
}

function issueUrl(value: string): { url: string; number: number } {
  const url = value.trim();
  const match = /^https:\/\/github\.com\/tryapparent\/apparent\/issues\/([1-9][0-9]*)$/.exec(url);
  const number = match ? Number(match[1]) : NaN;
  if (!Number.isSafeInteger(number)) execution('GitHub returned an unexpected issue URL; inspect the repository before retrying.');
  return { url, number };
}

async function gh(args: string[]): Promise<string> {
  let result: ChildResult;
  try { result = await runGh(args); }
  catch (error) { execution(error instanceof Error ? error.message : 'GitHub CLI failed; inspect the repository before retrying.'); }
  if (result!.code !== 0) execution('GitHub CLI failed without a verified result. Check authentication and repository access; inspect the repository before retrying because an issue may already exist.');
  try { return new TextDecoder('utf-8', { fatal: true }).decode(result!.stdout); }
  catch { execution('GitHub CLI returned non-UTF-8 output; inspect the repository before retrying.'); }
}

async function submit(title: string, body: string): Promise<Record<string, unknown>> {
  const created = await gh(['issue', 'create', '--repo', REPOSITORY, '--title', title, '--body', body]);
  const { url, number } = issueUrl(created);
  let readbackText: string;
  try { readbackText = await gh(['issue', 'view', url, '--repo', REPOSITORY, '--json', 'url,number,title,body']); }
  catch (error) {
    execution(`Issue may have been created at ${url}, but readback failed; inspect it before retrying. ${error instanceof Error ? error.message : ''}`.trim());
  }
  let issue: any;
  try { issue = JSON.parse(readbackText!); }
  catch { execution(`Issue may exist at ${url}, but readback was malformed; inspect it before retrying.`); }
  if (!issue || issue.url !== url || issue.number !== number || issue.title !== title || issue.body !== body)
    execution(`Issue exists at ${url}, but readback did not match the submitted identity and text; inspect it before retrying.`);
  return { repository: REPOSITORY, title, body, submitted: true, url, number };
}

export async function issueCommand(args: string[]): Promise<'help' | Record<string, unknown>> {
  const options = parse(args);
  if (options === 'help') return 'help';
  const body = options.body ?? await readBody(options.bodyFile!);
  validate(options.title, body);
  if (options.submit) return submit(options.title, body);
  return { repository: REPOSITORY, title: options.title, body, submitted: false };
}
