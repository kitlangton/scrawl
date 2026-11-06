import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { APP_SHELL_API, REACT_VERSION, parseStreamManifest } from '@apparent/app-contracts';
import { digest, readRegular } from '../scripts/artifacts.ts';
import { root } from '../scripts/build.ts';
import { checkInstalled } from '../scripts/check-installed.ts';

const external = ['react', 'react/jsx-runtime', 'react-dom/client'];

type Contribution = {
  id: string;
  label: string;
  description: string;
  setting: { kind: 'checkbox'; label: string; initial?: boolean } |
    { kind: 'select'; label: string; initial: string; options: string[] };
};

/** Write tiny synthetic producer sources into a test's disposable directory. */
export async function buildSyntheticStream(source: string, output: string, contribution: Contribution) {
  await checkInstalled();
  await mkdir(source, { recursive: true });
  await mkdir(output, { recursive: true });
  await writeFile(join(source, 'stream.json'), JSON.stringify({
    id: contribution.id, label: contribution.label, version: '1.0.0',
  }));
  const setting = contribution.setting.kind === 'checkbox'
    ? `const [checked, setChecked] = React.useState(() => localStorage.getItem(${JSON.stringify(`synthetic.${contribution.id}.checked`)}) === 'true');
      return <label><input type="checkbox" checked={checked} onChange={event => { localStorage.setItem(${JSON.stringify(`synthetic.${contribution.id}.checked`)}, String(event.target.checked)); setChecked(event.target.checked); }} />${contribution.setting.label}</label>;`
    : `const [value, setValue] = React.useState(() => localStorage.getItem(${JSON.stringify(`synthetic.${contribution.id}.value`)}) ?? ${JSON.stringify(contribution.setting.initial)});
      return <label>${contribution.setting.label}<select value={value} onChange={event => { localStorage.setItem(${JSON.stringify(`synthetic.${contribution.id}.value`)}, event.target.value); setValue(event.target.value); }}>${contribution.setting.options.map(option => `<option value=${JSON.stringify(option)}>${option}</option>`).join('')}</select></label>;`;
  const sourceText = `import * as React from 'react';
function Screen() { return <section><h2>${contribution.label} screen</h2><p>${contribution.description}</p></section>; }
function Settings() { ${setting} }
export default { id: ${JSON.stringify(contribution.id)}, Screen, Settings };
`;
  await writeFile(join(source, 'index.tsx'), sourceText);
  const result = await build({ bundle: true, format: 'esm', platform: 'browser', target: 'es2022',
    jsx: 'automatic', nodePaths: [join(root, 'node_modules')],
    define: { 'process.env.NODE_ENV': '"production"' }, legalComments: 'inline',
    entryPoints: [join(source, 'index.tsx')], outfile: join(output, 'index.js'), external, metafile: true });
  for (const file of Object.values(result.metafile!.outputs)) {
    assert(file.imports.every(item => external.includes(item.path)), 'Unbundled synthetic stream dependency');
  }
  const manifest = parseStreamManifest({ id: contribution.id, label: contribution.label, version: '1.0.0',
    schemaVersion: 1, shellApi: APP_SHELL_API, reactVersion: REACT_VERSION,
    entry: 'index.js', sha256: digest(await readRegular(join(output, 'index.js'))) });
  await writeFile(join(output, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
  return manifest;
}

/** Disposable compiled owner consumer for real main/preload/renderer bridge tests. */
export async function buildSyntheticProgressStream(source: string, output: string) {
  await checkInstalled();
  await mkdir(source, { recursive: true });
  await mkdir(output, { recursive: true });
  await writeFile(join(source, 'stream.json'), JSON.stringify({ id: 'onboarding', label: 'Onboarding', version: '1.0.0' }));
  await writeFile(join(source, 'index.tsx'), `import * as React from 'react';
const initial = { schema: 1, status: 'new', stage: 'welcome', attemptId: null, admissionId: null,
  practiceBaselineIds: [], legacy: { tutorial: null, relaunchedAt: null, reopenFix: false, shortcutChosen: false, firstCaptureCompleted: false }, manualPasteConfirmed: false };
function Screen({hostDependencies}: {hostDependencies?: any}) {
  const port = hostDependencies?.onboarding?.progress;
  const [snapshot, setSnapshot] = React.useState<any>(null);
  const [message, setMessage] = React.useState('idle');
  const read = async () => { try { const next = await port.read(); setSnapshot(next); setMessage('read:' + next.revision + ':' + next.value.stage); }
    catch { setMessage('read-unavailable'); } };
  const save = async (kind: string) => {
    if (!snapshot && kind !== 'unauthorized') { setMessage('missing-read'); return; }
    try {
      setMessage(kind === 'delayed' ? 'saving-delayed' : 'saving');
      let result: any;
      if (kind === 'invalid') {
        const invalid = Object.defineProperty({}, 'bad', { enumerable: true, get() { throw new Error('getter'); } });
        await port.compareAndSwap(snapshot.revision, invalid); setMessage('invalid-dispatched'); return;
      }
      if (kind === 'unauthorized') {
        result = await port.compareAndSwap(1, initial);
      } else {
        const expected = kind === 'conflict' ? Number(snapshot.revision) - 1 : snapshot.revision;
        result = await port.compareAndSwap(expected, snapshot.value);
      }
      setMessage(result.status + (result.revision === undefined ? '' : ':' + result.revision));
      if (result.status === 'saved') setSnapshot({...snapshot, revision: result.revision});
      if (result.status === 'conflict') setSnapshot({revision: result.revision, value: result.current});
    } catch { setMessage(kind === 'invalid' ? 'invalid-rejected-locally' : 'cas-unavailable'); }
  };
  const extraArguments = async () => {
    try { await (port.read as any)('arbitrary-owner'); setMessage('extra-read-dispatched'); }
    catch {
      try { await (port.compareAndSwap as any)(snapshot?.revision ?? 0, initial, 'extra'); setMessage('extra-cas-dispatched'); }
      catch { setMessage('extra-rejected-locally'); }
    }
  };
  return <section><h2>Onboarding bridge test</h2><output data-testid="bridge-state">{message}</output>
    <output data-testid="bridge-revision">{String(snapshot?.revision ?? '')}</output>
    <button onClick={() => read()}>Read progress</button><button onClick={() => read()}>Fail read</button>
    <button onClick={extraArguments}>Extra arguments</button><button onClick={() => save('unauthorized')}>Attempt direct CAS</button>
    <button onClick={() => save('saved')}>Save progress</button>
    <button onClick={() => save('conflict')}>Save stale</button><button onClick={() => save('invalid')}>Malformed progress</button>
    <button onClick={() => save('uncertain')}>Save uncertain</button><button onClick={() => save('malformed-result')}>Malformed result</button>
    <button onClick={() => save('delayed')}>Save delayed</button></section>;
}
function Settings({hostDependencies}: {hostDependencies?: any}) { return <Screen hostDependencies={hostDependencies} />; }
export default { id: 'onboarding', Screen, Settings };\n`);
  const result = await build({ bundle: true, format: 'esm', platform: 'browser', target: 'es2022', jsx: 'automatic',
    nodePaths: [join(root, 'node_modules')], define: { 'process.env.NODE_ENV': '"production"' }, legalComments: 'inline',
    entryPoints: [join(source, 'index.tsx')], outfile: join(output, 'index.js'), external });
  assert(result.outputFiles === undefined, 'Synthetic contribution build emitted files unexpectedly');
  const manifest = parseStreamManifest({ id: 'onboarding', label: 'Onboarding', version: '1.0.0', schemaVersion: 1,
    shellApi: APP_SHELL_API, reactVersion: REACT_VERSION, entry: 'index.js', sha256: digest(await readRegular(join(output, 'index.js'))) });
  await writeFile(join(output, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
  return manifest;
}
