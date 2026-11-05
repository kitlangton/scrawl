import { mkdtemp, mkdir, rm, writeFile, chmod, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, delimiter } from 'node:path';
import { spawnSync } from 'node:child_process';
import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { runGh } from '../dist/issue.js';

const cli = new URL('../dist/cli.js', import.meta.url).pathname;
const url = 'https://github.com/tryapparent/apparent/issues/123';
const title = 'Unicode 🐸 "quoted" title';
const body = 'Line one\n\nMarkdown `code` and Unicode 🐸\n';

async function fixture(root: string) {
  const bin = join(root, 'bin'); await mkdir(bin);
  const log = join(root, 'argv.bin');
  const expected = join(root, 'readback.json');
  await writeFile(expected, JSON.stringify({ url, number: 123, title, body }));
  const gh = join(bin, 'gh');
  await writeFile(gh, `#!/bin/sh
printf '%s\\000' "$@" >> "$FIXTURE_ARGV_LOG"
printf 'env:%s|%s|%s\\000' "$GH_HOST" "$GH_PROMPT_DISABLED" "$GH_PAGER" >> "$FIXTURE_ARGV_LOG"
printf 'PRIVATE_REQUEST_DIAGNOSTIC\\n' >&2
if [ "$1" = issue ] && [ "$2" = create ]; then
  case "$FIXTURE_MODE" in
    create-fail) exit 9 ;;
    bad-url) printf '%s\\n' 'https://evil.invalid/issues/123'; exit 0 ;;
  esac
  printf '%s\\n' '${url}'
  exit 0
fi
if [ "$1" = issue ] && [ "$2" = view ]; then
  case "$FIXTURE_MODE" in
    read-fail) exit 8 ;;
    malformed) printf '{not-json'; exit 0 ;;
    mismatch) printf '%s\\n' '{"url":"${url}","number":123,"title":"wrong","body":"wrong"}'; exit 0 ;;
  esac
  cat "$FIXTURE_READBACK"
  exit 0
fi
exit 7
`);
  await chmod(gh, 0o755);
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    PATH: `${bin}${delimiter}${process.env.PATH ?? ''}`,
    GH_HOST: 'attacker.example', GH_PROMPT_DISABLED: '0', GH_PAGER: 'less',
    FIXTURE_ARGV_LOG: log, FIXTURE_READBACK: expected, FIXTURE_MODE: 'success',
    GH_TOKEN: undefined, GITHUB_TOKEN: undefined,
  };
  return { env, log, bin };
}

test('issue preview, help, and invalid options never call gh', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-issue-'));
  try {
    const f = await fixture(root);
    const run = (...args: string[]) => spawnSync(process.execPath, [cli, 'issue', ...args], { encoding: 'utf8', env: f.env, cwd: root });
    const preview = run('--title', title, '--body', body);
    assert.equal(preview.status, 0); assert.equal(preview.stderr, '');
    assert.deepEqual(JSON.parse(preview.stdout), { repository: 'tryapparent/apparent', title, body, submitted: false });
    for (const args of [
      ['--help'], ['--title', title, '--body', body, '--unknown'],
      ['--title', title, '--title', title, '--body', body],
      ['--title', title, '--body', body, '--body-file', 'unused'],
      ['--title', title, '--body', body, '--submit', '--submit'],
      ['--body', body], ['--title', title],
      ['--title', title, '--body', '   '], ['--title', 'A\nB', '--body', body],
      ['--title', 'A\u0085B', '--body', body], ['--title', 'x'.repeat(257), '--body', body],
      ['--title', title, '--body', `x${'x'.repeat(60_000)}`], ['--title', title, '--body', '--submit'],
    ]) {
      const result = run(...args);
      if (args[0] === '--help') { assert.equal(result.status, 0); assert.match(result.stdout, /human's approval/); }
      else { assert.equal(result.status, 2, `${args.join(' ')}: ${result.stderr}`); assert.equal(result.stdout, ''); }
    }
    assert.equal(await readFile(f.log).catch(() => Buffer.alloc(0)).then(bytes => bytes.length), 0);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('issue body-file preserves exact UTF-8 text and refuses FIFO inputs promptly', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-issue-file-'));
  try {
    const f = await fixture(root);
    const path = join(root, 'body draft.md'); await writeFile(path, body);
    const preview = spawnSync(process.execPath, [cli, 'issue', '--title', title, '--body-file', path], { encoding: 'utf8', env: f.env });
    assert.equal(preview.status, 0); assert.equal(JSON.parse(preview.stdout).body, body);
    await writeFile(path, `\uFEFF${body}`);
    const withBom = spawnSync(process.execPath, [cli, 'issue', '--title', title, '--body-file', path], { encoding: 'utf8', env: f.env });
    assert.equal(withBom.status, 0); assert.equal(JSON.parse(withBom.stdout).body, `\uFEFF${body}`);
    const fifo = join(root, 'no-writer.fifo');
    const created = spawnSync('mkfifo', [fifo]); assert.equal(created.status, 0);
    const started = Date.now();
    const refused = spawnSync(process.execPath, [cli, 'issue', '--title', title, '--body-file', fifo], { encoding: 'utf8', env: f.env, timeout: 2_000 });
    assert.equal(refused.status, 1); assert.match(refused.stderr, /regular file/); assert.ok(Date.now() - started < 2_000);
    const invalidUtf8 = join(root, 'invalid-utf8.md'); await writeFile(invalidUtf8, Buffer.from([0xff, 0xfe]));
    const unreadable = spawnSync(process.execPath, [cli, 'issue', '--title', title, '--body-file', invalidUtf8], { encoding: 'utf8', env: f.env });
    assert.equal(unreadable.status, 1); assert.match(unreadable.stderr, /valid UTF-8/);
    const nulBody = join(root, 'nul.md'); await writeFile(nulBody, 'before\0after');
    const nul = spawnSync(process.execPath, [cli, 'issue', '--title', title, '--body-file', nulBody], { encoding: 'utf8', env: f.env });
    assert.equal(nul.status, 2); assert.match(nul.stderr, /contain no NUL/);
    const scalarTitle = '🐸'.repeat(256), scalarBody = '🐸'.repeat(15_000);
    const boundary = spawnSync(process.execPath, [cli, 'issue', '--title', scalarTitle, '--body', scalarBody], { encoding: 'utf8', env: f.env });
    assert.equal(boundary.status, 0, boundary.stderr);
    assert.equal(JSON.parse(boundary.stdout).title, scalarTitle); assert.equal(JSON.parse(boundary.stdout).body, scalarBody);
    await writeFile(path, scalarBody);
    const fileBoundary = spawnSync(process.execPath, [cli, 'issue', '--title', title, '--body-file', path], { encoding: 'utf8', env: f.env });
    assert.equal(fileBoundary.status, 0, fileBoundary.stderr);
    assert.equal(JSON.parse(fileBoundary.stdout).body, scalarBody);
    await writeFile(path, `${scalarBody}x`);
    const oversized = spawnSync(process.execPath, [cli, 'issue', '--title', title, '--body-file', path, '--submit'], { encoding: 'utf8', env: f.env });
    assert.equal(oversized.status, 1); assert.equal(oversized.stdout, '');
    assert.equal(await readFile(f.log).catch(() => Buffer.alloc(0)).then(bytes => bytes.length), 0);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('explicit submission pins child host and repository, preserves argv, and verifies readback', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-issue-submit-'));
  try {
    const f = await fixture(root);
    const result = spawnSync(process.execPath, [cli, 'issue', '--title', title, '--body', body, '--submit'], { encoding: 'utf8', env: f.env, cwd: root });
    assert.equal(result.status, 0, result.stderr); assert.equal(result.stderr, '');
    assert.deepEqual(JSON.parse(result.stdout), { repository: 'tryapparent/apparent', title, body, submitted: true, url, number: 123 });
    const fields = (await readFile(f.log)).toString().split('\0');
    assert.deepEqual(fields.slice(0, 10), ['issue', 'create', '--repo', 'tryapparent/apparent', '--title', title, '--body', body, 'env:github.com|1|cat', 'issue']);
    assert.equal(fields[10], 'view'); assert.equal(fields[11], url); assert.equal(fields[12], '--repo');
    assert.equal(fields[13], 'tryapparent/apparent'); assert.equal(fields[14], '--json'); assert.equal(fields[15], 'url,number,title,body');
    assert.ok(!result.stdout.includes('PRIVATE_REQUEST_DIAGNOSTIC'));
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('create and readback failures never expose gh stderr, claim success, or retry create', async () => {
  for (const mode of ['create-fail', 'bad-url', 'read-fail', 'malformed', 'mismatch']) {
    const root = await mkdtemp(join(tmpdir(), `agent-issue-${mode}-`));
    try {
      const f = await fixture(root);
      f.env.FIXTURE_MODE = mode;
      const result = spawnSync(process.execPath, [cli, 'issue', '--title', title, '--body', body, '--submit'], { encoding: 'utf8', env: f.env });
      assert.equal(result.status, 1, `${mode}: ${result.stderr}`); assert.equal(result.stdout, '');
      assert.ok(!result.stderr.includes('PRIVATE_REQUEST_DIAGNOSTIC')); assert.ok(!result.stderr.includes(body));
      if (['read-fail', 'malformed', 'mismatch'].includes(mode)) assert.ok(result.stderr.includes(url));
      const fields = (await readFile(f.log)).toString().split('\0');
      assert.equal(fields.filter(value => value === 'create').length, 1, `${mode} repeated create`);
    } finally { await rm(root, { recursive: true, force: true }); }
  }
});

test('gh transport kills process groups on timeout and output overflow', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-issue-transport-'));
  try {
    const f = await fixture(root);
    const hanging = join(f.bin, 'gh');
    await writeFile(hanging, '#!/bin/sh\nsleep 10 &\nwait\n'); await chmod(hanging, 0o755);
    const start = Date.now();
    const originalPath = process.env.PATH;
    process.env.PATH = f.env.PATH;
    try { await assert.rejects(runGh(['issue', 'create'], 100), /deadline/); }
    finally { if (originalPath === undefined) delete process.env.PATH; else process.env.PATH = originalPath; }
    assert.ok(Date.now() - start < 2_000);
    await writeFile(hanging, '#!/bin/sh\nyes synthetic-output\n'); await chmod(hanging, 0o755);
    process.env.PATH = f.env.PATH;
    try { await assert.rejects(runGh(['issue', 'create'], 2_000), /512 KiB/); }
    finally { if (originalPath === undefined) delete process.env.PATH; else process.env.PATH = originalPath; }
  } finally { await rm(root, { recursive: true, force: true }); }
});
