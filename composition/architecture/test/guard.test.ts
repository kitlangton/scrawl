import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { execFileSync, spawnSync } from 'node:child_process';
import { checkArchitecture } from '../src/guard.ts';
const gitEnvironment = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('GIT_')));

function fixture(files: Record<string,string>, run: (root: string) => void): void {
  const root = mkdtempSync(path.join(tmpdir(), 'apparent-arch-'));
  try {
    for (const [name, value] of Object.entries(files)) {
      const file = path.join(root,name); mkdirSync(path.dirname(file),{recursive:true}); writeFileSync(file,value);
    }
    execFileSync('git',['init','-q',root], { env: gitEnvironment }); execFileSync('git',['-C',root,'add','.'], { env: gitEnvironment });
    run(root);
  } finally { rmSync(root,{recursive:true,force:true}); }
}
const engine = JSON.stringify({name:'@apparent/engine-sdk',exports:{'.':'./dist/index.js','./testing':'./dist/testing.js'}});
const record = JSON.stringify({name:'@apparent/record',exports:{'.':'./dist/index.js','./fixtures':'./dist/fixtures.js'}});

test('allows public APIs and owner-local internals', () => fixture({
  'engine/sdk/package.json': engine, 'record/package.json': record,
  'shared/ui/package.json': JSON.stringify({ name: '@apparent/ui', exports: { './button': './src/button.tsx' } }),
  'record/src/index.ts': "import { x } from '@apparent/engine-sdk'; import { y } from './local.js'; import '@apparent/ui/button'; export { y };",
  'record/src/local.ts': 'export const y = 1;',
}, root => assert.deepEqual(checkArchitecture(root), [])));

test('Agent consumes the published public Record package while private paths and reverse edges stay blocked', () => {
  const recordPackage = JSON.stringify({ name: '@apparent/record', exports: {
    '.': './dist/index.js', './host': './dist/backend/host/index.js',
    './host/capture2': './dist/backend/host/capture2.js',
  } });
  const agentPackage = JSON.stringify({ name: '@apparent/agent', dependencies: {
    '@apparent/record': 'file:vendor/record-0.1.0-89fcd0a.tgz',
  } });
  const source = "import '@apparent/record'; import '@apparent/record/host'; import '@apparent/record/host/capture2';";
  fixture({ 'record/package.json': recordPackage, 'agent/package.json': agentPackage,
    'agent/src/index.ts': source }, root => assert.deepEqual(checkArchitecture(root, ['agent']), []));
  fixture({ 'record/package.json': recordPackage, 'agent/package.json': agentPackage,
    'agent/src/index.ts': source + " import '@apparent/record/src/private.js';" }, root => {
    assert(checkArchitecture(root, ['agent']).some(error => error.includes('private or undeclared')));
  });
  fixture({ 'record/package.json': recordPackage, 'agent/package.json': agentPackage,
    'agent/src/index.ts': "import '../../record/src/private.js';", 'record/src/private.ts': 'export {};',
  }, root => assert(checkArchitecture(root, ['agent']).some(error => error.includes('cross-owner path agent -> record'))));
  fixture({ 'record/package.json': recordPackage, 'agent/package.json': agentPackage,
    'agent/tsconfig.json': JSON.stringify({ compilerOptions: { baseUrl: '..', paths: { '@apparent/record': ['record/src/private.ts'] } } }),
    'agent/src/index.ts': "import '@apparent/record';", 'record/src/private.ts': 'export {};',
  }, root => assert(checkArchitecture(root, ['agent']).some(error => error.includes('tsconfig alias crosses agent -> record'))));
  fixture({ 'record/package.json': JSON.stringify({ name: '@apparent/record', dependencies: { '@apparent/agent': '0.1.0' } }),
    'agent/package.json': agentPackage,
  }, root => assert(checkArchitecture(root, ['record']).some(error => error.includes('dependency @apparent/agent violates record -> agent'))));
});

test('Agent public NativeHost edge is limited to root and clipboard and never reversed', () => {
  const host = JSON.stringify({ name: '@apparent/native-host', exports: {
    '.': './dist/client.js', './clipboard': './dist/clipboard.js', './pen': './dist/pen.js',
  } });
  const agent = JSON.stringify({ name: '@apparent/agent', dependencies: {
    '@apparent/native-host': 'file:vendor/native-host.tgz',
  } });
  const base = { 'composition/native-host/package.json': host, 'agent/package.json': agent };
  fixture({ ...base, 'agent/src/copy.ts': "import '@apparent/native-host'; import '@apparent/native-host/clipboard';" },
    root => assert.deepEqual(checkArchitecture(root, ['agent']), []));
  fixture({ ...base, 'agent/package.json': JSON.stringify({ name: '@apparent/agent', dependencies: {
    '@apparent/native-host': 'file:../composition/native-host',
  } }) }, root => assert.deepEqual(checkArchitecture(root, ['agent']), []));
  fixture({ ...base, 'composition/private-host/package.json': host,
    'agent/package.json': JSON.stringify({ name: '@apparent/agent', dependencies: {
      '@apparent/native-host': 'file:../composition/private-host',
    } }),
  }, root => assert(checkArchitecture(root, ['agent']).some(error => error.includes('canonical public package root'))));
  for (const spec of ['@apparent/native-host/pen', '@apparent/native-host/src/client.ts', '@apparent/other-host']) {
    fixture({ ...base, 'composition/other/package.json': JSON.stringify({ name: '@apparent/other-host', exports: { '.': './index.ts' } }),
      'agent/src/copy.ts': `import '${spec}';` }, root => {
      assert(checkArchitecture(root, ['agent']).some(error => error.includes('forbidden package dependency')));
    });
  }
  fixture({ ...base, 'agent/src/copy.ts': "import '../../composition/native-host/src/client.ts';" },
    root => assert(checkArchitecture(root, ['agent']).some(error => error.includes('cross-owner path'))));
  fixture({ ...base, 'agent/src/copy.ts': "import '@apparent/native-host';",
    'agent/tsconfig.json': JSON.stringify({ compilerOptions: { baseUrl: '..', paths: {
      '@apparent/native-host': ['composition/native-host/src/client.ts'],
    } } }), 'composition/native-host/src/client.ts': 'export {};',
  }, root => assert(checkArchitecture(root, ['agent']).some(error => error.includes('tsconfig alias crosses'))));
  fixture({ ...base, 'composition/native-host/package.json': JSON.stringify({ name: '@apparent/native-host',
    dependencies: { '@apparent/agent': '0.1.0' } }), 'composition/native-host/src/client.ts': "import '@apparent/agent';",
  }, root => {
    const errors = checkArchitecture(root, ['composition']);
    assert(errors.some(error => error.includes('dependency @apparent/agent violates')));
    assert(errors.some(error => error.includes('forbidden package dependency')));
  });
  fixture({ ...base, 'agent/package.json': JSON.stringify({ name: '@apparent/agent', exports: { '.': './dist/index.js' } }),
    'composition/app/src/main.ts': "import '@apparent/agent';",
  }, root => assert.deepEqual(checkArchitecture(root, ['composition']), []));
  fixture({ ...base, 'agent/package.json': JSON.stringify({ name: '@apparent/agent', dependencies: {
    '@apparent/other-host': '0.1.0',
  } }), 'composition/other/package.json': JSON.stringify({ name: '@apparent/other-host' }),
  }, root => assert(checkArchitecture(root, ['agent']).some(error => error.includes('dependency @apparent/other-host violates'))));
});

test('Connect client cannot import Record private implementation', () => fixture({
  'connect/delivery/client/package.json': JSON.stringify({ name: '@apparent/connect-client', exports: { '.': './dist/index.js' } }),
  'connect/delivery/client/src/index.ts': "import '../../../../record/src/private.js';",
  'record/src/private.ts': 'export const internal = true;',
}, root => {
  const errors = checkArchitecture(root, ['connect']);
  assert(errors.some(error => error.includes('cross-owner path connect -> record')));
}));

test('shared cannot hide product imports behind a computed loader or forged package name', () => fixture({
  'shared/contracts/src/index.ts': 'import(destination); require(destination);',
  'shared/forged/package.json': JSON.stringify({ name: '@apparent/record', exports: { '.': './index.ts' } }),
}, root => {
  const errors = checkArchitecture(root);
  assert(errors.some(error => error.includes('computed dynamic import')));
  assert(errors.some(error => error.includes('computed require')));
  assert(errors.some(error => error.includes('public package identity')));
}));

test('rejects relative cross-owner import and re-export', () => fixture({
  'engine/sdk/package.json': engine,
  'record/src/index.ts': "import '../../engine/sdk/src/index.js';\nexport { x } from '../../engine/sdk/src/index.js';",
}, root => { const errors=checkArchitecture(root); assert(errors.some(x=>x.includes('cross-owner path'))); assert(errors.length >= 2); }));

test('rejects aliases that escape an owner', () => fixture({
  'engine/sdk/package.json': engine, 'record/tsconfig.json': JSON.stringify({compilerOptions:{baseUrl:'..',paths:{'@engine/*':['engine/sdk/src/*']}}}),
  'record/src/index.ts': "import '@engine/index.js';",
}, root => assert(checkArchitecture(root).some(x=>x.includes('tsconfig alias')))));

test('checks a package-looking alias before accepting its public export', () => fixture({
  'record/tsconfig.json': JSON.stringify({compilerOptions:{baseUrl:'..',paths:{'@apparent/engine-sdk':['agent/src/index.ts']}}}),
  'record/src/index.ts': "import '@apparent/engine-sdk';",
}, root => assert(checkArchitecture(root).some(x=>x.includes('tsconfig alias crosses record -> agent')))));

test('does not apply unrelated aliases globally', () => fixture({
  'engine/tsconfig.json': JSON.stringify({compilerOptions:{baseUrl:'..',paths:{'@private/*':['agent/src/*']}}}),
  'record/src/index.ts': "import '@private/hidden';",
}, root => assert.deepEqual(checkArchitecture(root), [])));

test('resolves inherited path mappings from their declaring config directory', () => fixture({
  'tsconfig.base.json': JSON.stringify({ compilerOptions: { paths: { '@hidden/*': ['engine/sdk/src/*'] } } }),
  'record/tsconfig.json': JSON.stringify({ extends: '../tsconfig.base.json' }),
  'record/src/index.ts': "import '@hidden/private';",
}, root => assert(checkArchitecture(root).some(error => error.includes('alias crosses record -> engine')))));

test('baseUrl-only bare imports cannot reach another owner implementation', () => fixture({
  'record/tsconfig.json': JSON.stringify({ compilerOptions: { baseUrl: '..', moduleResolution: 'NodeNext', module: 'NodeNext' } }),
  'record/src/index.ts': "import 'engine/sdk/src/private.js';",
  'engine/sdk/src/private.ts': 'export const privateImplementation = true;',
  'record/faux/tsconfig.json': JSON.stringify({ compilerOptions: { baseUrl: '../../engine/sdk/src', moduleResolution: 'NodeNext', module: 'NodeNext' } }),
  'record/faux/index.ts': "import '@apparent/engine-sdk';",
  'engine/sdk/package.json': engine,
  'engine/sdk/src/@apparent/engine-sdk.ts': 'export const hiddenByPackageName = true;',
}, root => assert.equal(checkArchitecture(root).filter(error => error.includes('resolved bare import crosses owner boundary')).length, 2)));

test('rejects private package subpaths and computed imports', () => fixture({
  'engine/sdk/package.json': engine,
  'record/src/index.ts': "import '@apparent/engine-sdk/src/private.js'; import(name);",
}, root => { const errors=checkArchitecture(root); assert(errors.some(x=>x.includes('private or undeclared'))); assert(errors.some(x=>x.includes('computed dynamic import'))); }));

test('rejects reversed dependencies and cyclic owner edge', () => fixture({
  'engine/sdk/package.json': engine, 'engine/package.json': JSON.stringify({name:'@apparent/engine',dependencies:{'@apparent/record':'1.0.0'}}),
  'engine/sdk/src/index.ts': "import '@apparent/record';",
}, root => { const errors=checkArchitecture(root); assert(errors.some(x=>x.includes('dependency @apparent/record'))); assert(errors.some(x=>x.includes('forbidden package dependency'))); }));

test('peer dependencies and npm aliases cannot hide a reversed product edge', () => fixture({
  'engine/sdk/package.json': JSON.stringify({ name: '@apparent/engine-sdk',
    peerDependencies: { '@apparent/record': '0.1.0' },
    dependencies: { hidden: 'npm:@apparent/record@0.1.0' } }),
  'record/package.json': record,
}, root => {
  const errors = checkArchitecture(root);
  assert(errors.some(error => error.includes('dependency @apparent/record violates engine -> record')));
  assert(errors.some(error => error.includes('canonical public name')));
}));

test('owner-scoped CI does not make an unrelated stream fail', () => fixture({
  'agent/src/index.ts': "const destination = '../record/src/index.js'; import(destination);",
  'record/src/index.ts': 'export const ready = true;',
}, root => {
  assert.deepEqual(checkArchitecture(root, ['record']), []);
  assert(checkArchitecture(root, ['agent']).some(error => error.includes('computed dynamic import')));
}));

test('bounded packed SDK fixture imports must match declared public export targets', () => {
  const source = [
    "import { pathToFileURL } from 'node:url';",
    'async function packedSdk() {',
    'const root = process.env.APPARENT_ENGINE_SDK_CONSUMER;',
    'const clientPath = `${root}/node_modules/@apparent/engine-sdk/dist/index.js`;',
    'return import(pathToFileURL(clientPath).href);',
    '}',
  ].join('\n');
  const policy = JSON.stringify({ schema: 1, fixtureImports: [{ file: 'test/sdk-adapter.test.ts', function: 'packedSdk', env: 'APPARENT_ENGINE_SDK_CONSUMER', package: '@apparent/engine-sdk', exports: ['.'] }] });
  fixture({ 'engine/sdk/package.json': engine, 'record/package.json': record, 'record/architecture.json': policy, 'record/test/sdk-adapter.test.ts': source }, root => {
    assert.deepEqual(checkArchitecture(root, ['record']), []);
    writeFileSync(path.join(root, 'record/test/sdk-adapter.test.ts'), source.replace('dist/index.js', 'src/private.ts'));
    assert(checkArchitecture(root, ['record']).some(error => error.includes('computed dynamic import')));
    writeFileSync(path.join(root, 'record/test/sdk-adapter.test.ts'), source.replace('APPARENT_ENGINE_SDK_CONSUMER', 'UNBOUNDED_ROOT'));
    assert(checkArchitecture(root, ['record']).some(error => error.includes('computed dynamic import')));
    writeFileSync(path.join(root, 'record/test/other.test.ts'), source);
    assert(checkArchitecture(root, ['record']).some(error => error.includes('other.test.ts')));
    writeFileSync(path.join(root, 'record/architecture.json'), policy.replace('test/sdk-adapter.test.ts', 'src/index.ts'));
    assert(checkArchitecture(root, ['record']).some(error => error.includes('invalid bounded fixture-import policy')));
  });
});

test('checks nested package manifests and validates local package targets', () => fixture({
  'record/package.json': JSON.stringify({name:'@apparent/record',dependencies:{'@apparent/engine-sdk':'file:../agent'}}),
  'engine/sdk/package.json': engine,
  'agent/package.json': JSON.stringify({name:'@apparent/agent'}),
}, root => {
  const errors=checkArchitecture(root);
  assert(errors.some(x=>x.includes('dependency @apparent/engine-sdk violates record -> agent')));
  assert(errors.some(x=>x.includes('targets package @apparent/agent')));
}));

test('rejects escaping paths and symlinks that cross owners', () => fixture({
  'record/src/index.ts': "import '../../../outside.ts'; import './linked.ts';",
  'record/src/owned.ts': 'export const owned = true;',
  'agent/src/private.ts': 'export const privateValue = true;',
}, root => {
  symlinkSync(path.join(root,'agent/src/private.ts'),path.join(root,'record/src/linked.ts'));
  const errors=checkArchitecture(root);
  assert(errors.some(x=>x.includes('outside the repository')));
  assert(errors.some(x=>x.includes('cross-owner path record -> agent')));
}));

test('discovers .mts files and filenames containing newlines', () => fixture({
  'record/src/line\nbreak.mts': "import '../../agent/src/private.js';",
  'agent/src/private.ts': 'export const x = 1;',
}, root => assert(checkArchitecture(root).some(x=>x.includes('cross-owner path')))));

test('Connect gateway and service share one owner while service imports cannot cross product boundaries',()=>{
 fixture({
  'connect/tooling/hosting/src/index.ts': "import service from '../../../service/src/worker.ts';",
  'connect/service/src/worker.ts': 'export default {};',
 },root=>assert.deepEqual(checkArchitecture(root,['connect']),[]));
 fixture({
  'connect/service/src/worker.ts': "import '../../../record/src/private.ts';",
  'record/src/private.ts': 'export const internal = true;',
 },root=>assert.ok(checkArchitecture(root,['connect']).some(error=>error.includes('connect -> record'))));
});

// CLI integration belongs with its locked TypeScript package, not dependency-free hook contracts.
test('CLI rejects a record-to-agent source edge', () => fixture({
  'record/src/index.ts': "import {value} from '../../agent/src/secret.ts'; void value;",
  'agent/src/secret.ts': 'export const value = 1;',
}, root => {
  const result = spawnSync(process.execPath, ['--experimental-strip-types',
    path.resolve(import.meta.dirname, '../src/cli.ts'), '--root', root, '--owner', 'record'],
    { encoding: 'utf8', env: gitEnvironment });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /cross-owner path/);
}));
