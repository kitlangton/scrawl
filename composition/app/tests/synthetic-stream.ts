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
