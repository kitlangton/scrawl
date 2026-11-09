import {test} from 'node:test';
import assert from 'node:assert/strict';
import {spawn, spawnSync} from 'node:child_process';
import {once} from 'node:events';
import {existsSync, mkdtempSync, readFileSync, rmSync, watch, writeFileSync} from 'node:fs';
import {join, resolve} from 'node:path';

const script = resolve('enablement/ci/root.ts');
const source = String.raw`
import {appendFileSync, existsSync, writeFileSync} from 'node:fs';
import {spawn} from 'node:child_process';
const [mode, action] = process.argv.slice(2);
const trace = value => appendFileSync(process.env.TRACE, JSON.stringify({value, pid:process.pid, parent:process.ppid})+'\n');
trace(action+':start');
if (action === 'test') {
  if (mode === 'compile-fails') {console.log(JSON.stringify({reason:'build-finished',success:false})); process.exit(19);}
  if (mode === 'compile-refused-zero') {console.log(JSON.stringify({reason:'build-finished',success:false})); process.exit(0);}
  if (mode === 'missing-barrier') {console.log('ordinary harness output'); process.exit(0);}
  writeFileSync(process.env.RUNNING, 'running');
  console.log('ordinary harness output');
  process.stdout.write(JSON.stringify({reason:'compiler-message',message:{rendered:'compiler diagnostic — preserved\n'}})+'\n');
  const barrier=JSON.stringify({reason:'build-finished',success:true})+'\n';
  process.stdout.write(barrier.slice(0,18));
  process.stdout.write(barrier.slice(18));
  if (mode === 'duplicate-barrier') process.stdout.write(barrier);
  if (mode === 'test-fails') {setTimeout(()=>process.exit(23),30);}
  else if (mode === 'test-signal') {setTimeout(()=>process.kill(process.pid,'SIGTERM'),30);}
  else if (mode === 'inherited-stdout') {
    const child=spawn(process.execPath,['-e',
      "require('fs').appendFileSync(process.env.TRACE,JSON.stringify({value:'descendant',pid:process.pid,parent:process.ppid})+'\\n');process.send('ready');process.disconnect();setInterval(()=>{},1000)"],
      {stdio:['ignore','inherit','inherit','ipc']});
    child.on('message',()=>process.exit(0));
  } else if (['build-fails','cancel','disconnect'].includes(mode)) setInterval(()=>{},1000);
  else {
    const timer=setInterval(()=>{
      if(existsSync(process.env.BUILT)){clearInterval(timer);trace('test:end');process.exit(0);}
    },5);
  }
} else {
  if(!existsSync(process.env.RUNNING)) throw Error('Build started before compilation barrier');
  if(mode==='build-fails') setTimeout(()=>process.exit(31),30);
  else if(['test-fails','test-signal','cancel','disconnect'].includes(mode)) setInterval(()=>{},1000);
  else setTimeout(()=>{process.kill(process.ppid,0);trace('build:end');writeFileSync(process.env.BUILT,'done');},30);
}
`;

function fixture(t: any, mode: string, spawnFailure = '') {
  const directory = mkdtempSync('/tmp/apparent-dev-root-overlap-');
  t.after(() => rmSync(directory, {recursive: true, force: true}));
  const cargo = join(directory, 'cargo.ts'), trace = join(directory, 'trace');
  writeFileSync(cargo, source);
  const commands = {test: ['node', '--experimental-strip-types', cargo, mode, 'test'],
    build: ['node', '--experimental-strip-types', cargo, mode, 'build']};
  if (spawnFailure) commands[spawnFailure as 'test' | 'build'] = [join(directory, 'nonexistent')];
  const child = spawn('node', ['--experimental-strip-types', script, JSON.stringify(commands)], {env: {
    ...process.env, TRACE: trace, RUNNING: join(directory, 'running'), BUILT: join(directory, 'built'),
    APPARENT_TIMING_FILE: join(directory, 'timing.jsonl'), GITHUB_SHA: 'synthetic-source',
  }, stdio: ['ignore', 'pipe', 'pipe']});
  let stdout = '', stderr = '';
  child.stdout.on('data', chunk => { stdout += chunk; }); child.stderr.on('data', chunk => { stderr += chunk; });
  const done = once(child, 'close').then(([code, signal]) => ({code, signal, stdout, stderr}));
  t.after(async () => {if (child.exitCode === null && child.signalCode === null) {child.kill('SIGTERM'); await done;}});
  const rows = () => existsSync(trace) ? readFileSync(trace, 'utf8').trim().split('\n').filter(Boolean).map(line => JSON.parse(line)) : [];
  const ready = (value: string) => new Promise<void>(resolve => {
    const watcher = watch(directory, () => {if (rows().some(row => row.value === value)) {watcher.close(); resolve();}});
    t.after(() => watcher.close());
    if (rows().some(row => row.value === value)) {watcher.close();resolve();}
  });
  return {directory, child, done, rows, ready};
}

function assertStopped(rows: any[]) {
  for (const {pid} of rows) {
    const state = spawnSync('ps', ['-o', 'stat=', '-p', String(pid)], {encoding: 'utf8'}).stdout.trim();
    assert.ok(!state || state.startsWith('Z'), `owned command still running: ${pid} ${state}`);
  }
}

test('actual commands overlap only after compilation, retain output/timings and require both successes', {timeout: 8000}, async t => {
  const f = fixture(t, 'success'); const result = await f.done;
  assert.equal(result.code, 0, result.stderr + result.stdout);
  const events = f.rows().map(row => row.value);
  assert.ok(events.indexOf('build:start') < events.indexOf('test:end'));
  assert.match(result.stdout, /ordinary harness output/); assert.match(result.stdout, /compiler diagnostic — preserved/);
  const timings = readFileSync(join(f.directory, 'timing.jsonl'), 'utf8').trim().split('\n').map(line => JSON.parse(line));
  assert.deepEqual(timings.map(row => row.label).sort(), ['rust-build', 'rust-test-compilation', 'rust-test-execution']);
  assert.ok(timings.every(row => row.code === 0 && row.sha === 'synthetic-source'));
  assertStopped(f.rows());
});

for (const [mode, expected] of [['compile-fails', 19], ['compile-refused-zero', 1], ['missing-barrier', 1]] as const)
  test(`${mode} cannot start the application build or pass`, {timeout: 8000}, async t => {
    const f = fixture(t, mode), result = await f.done;
    assert.equal(result.code, expected, result.stderr); assert.equal(f.rows().some(row => row.value === 'build:start'), false);
    assertStopped(f.rows());
  });

for (const [mode, expected] of [['test-fails', 23], ['build-fails', 31], ['test-signal', 143]] as const)
  test(`${mode} preserves failure and stops the other live command`, {timeout: 8000}, async t => {
    const f = fixture(t, mode), result = await f.done;
    assert.equal(result.code, expected, result.stderr + result.stdout); assertStopped(f.rows());
    assert.doesNotMatch(result.stdout, /both succeeded/);
  });

for (const command of ['test', 'build'])
  test(`${command} spawn failure fails closed`, {timeout: 8000}, async t => {
    const f = fixture(t, 'success', command), result = await f.done;
    assert.equal(result.code, 1); assert.match(result.stdout, /spawn failed/); assertStopped(f.rows());
  });

test('duplicate Cargo success does not duplicate the build', {timeout: 8000}, async t => {
  const f = fixture(t, 'duplicate-barrier'); assert.equal((await f.done).code, 0);
  assert.equal(f.rows().filter(row => row.value === 'build:start').length, 1);
});

test('leader exit does not await stdout EOF held by a descendant', {timeout: 8000}, async t => {
  const f = fixture(t, 'inherited-stdout'), result = await f.done;
  assert.equal(result.code, 0, result.stderr); assert.ok(f.rows().some(row => row.value === 'descendant'));
  assertStopped(f.rows());
});

for (const [mode, signal] of [['cancel', 'SIGTERM'], ['disconnect', 'SIGKILL']] as const)
  test(`${mode} cleans the anchored group while unrelated work survives`, {timeout: 8000}, async t => {
    const unrelated = spawn('node', ['-e', 'setInterval(()=>{},1000)'], {stdio: 'ignore'});
    const unrelatedDone = once(unrelated, 'exit');
    t.after(async () => {unrelated.kill(); await unrelatedDone;});
    const f = fixture(t, mode); await f.ready('build:start');
    f.child.kill(signal); const result = await f.done;
    if (mode === 'cancel') assert.equal(result.code, 143);
    else assert.equal(result.signal, 'SIGKILL');
    assertStopped(f.rows()); assert.equal(unrelated.exitCode, null); assert.equal(unrelated.signalCode, null);
  });

test('the real verifier adapter preserves default/focused/portable fallbacks and Cargo allocation', t => {
  const directory = mkdtempSync('/tmp/apparent-dev-root-adapter-');
  t.after(() => rmSync(directory, {recursive: true, force: true}));
  const result = spawnSync('python3', ['-c', String.raw`
import importlib.util,json,os,subprocess
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('verify','enablement/verification.py')
v=importlib.util.module_from_spec(spec);spec.loader.exec_module(v)
selection={'root':True,'engine':True,'browser':False,'focused':[]}
for jobs,requested,focused,execution in [('3',True,[],'platform'),('3',False,[],'platform'),('2',True,[],'platform'),('3',True,['capture::'],'platform'),('3',True,[],'portable')]:
 for mac in [True,False]:
  commands=[]
  def execute(command,**kwargs):
   commands.append(command)
   if command[0]=='sh': return subprocess.CompletedProcess(command,0 if mac else 1)
   return subprocess.CompletedProcess(command,0,stdout='test result: ok. 2 passed; 0 failed')
  with patch.object(v,'check_temp_environment'),patch.object(v.subprocess,'run',side_effect=execute),patch.dict(os.environ,{'T3_BUILD_JOBS':jobs,'APPARENT_TIMING_FILE':os.environ['APPARENT_ADAPTER_TIMING']}):
   v.run_checks(dict(selection,focused=focused),offline=False,execution=execution,overlap_root_build=requested)
  helpers=[c for c in commands if c[0]=='node']
  overlap=requested and int(jobs)>=3 and not focused and execution!='portable'
  assert bool(helpers)==overlap,(jobs,requested,focused,execution,commands)
  if overlap:
   payload=json.loads(helpers[0][-1]);prefix=['taskpolicy','-c','utility','cargo'] if mac else ['cargo']
   assert payload['test']==prefix+['test','-j',jobs,'--locked','--timings','--message-format=json-render-diagnostics','--','--test-threads=2'],payload
   assert payload['build']==prefix+['build','-j','1','--locked','--timings'],payload
   assert not any(c[0]=='cargo' and 'capture/engine/Cargo.toml' not in c for c in commands)
   assert any('capture/engine/Cargo.toml' in c for c in commands)
# A controller failure cannot admit Engine or downstream verification.
def failed(command,**kwargs):
 if command[0]=='node': raise subprocess.CalledProcessError(31,command)
 assert 'capture/engine/Cargo.toml' not in command
 return subprocess.CompletedProcess(command,1 if command[0]=='sh' else 0)
with patch.object(v,'check_temp_environment'),patch.object(v.subprocess,'run',side_effect=failed),patch.dict(os.environ,{'T3_BUILD_JOBS':'3','APPARENT_TIMING_FILE':os.environ['APPARENT_ADAPTER_TIMING']}):
 try:v.run_checks(selection,execution='platform',overlap_root_build=True)
 except subprocess.CalledProcessError as e:assert e.returncode==31
 else:raise AssertionError('failed overlap passed')
`], {encoding: 'utf8', env: {...process.env, APPARENT_ADAPTER_TIMING: join(directory, 'timings.jsonl')}});
  assert.equal(result.status, 0, result.stderr + result.stdout);
  assert.ok(existsSync(join(directory, 'timings.jsonl')));
  const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8');
  assert.match(workflow, /verification\.py --base "\$BASE" --run --online --execution platform --overlap-root-build/);
  assert.equal((workflow.match(/--overlap-root-build/g) ?? []).length, 1);
});

const cargoTest = process.env.APPARENT_ROOT_REAL_CARGO === '1' ? test : test.skip;
cargoTest('opt-in actual Cargo releases its compilation lock and builds an executable while its test is running', {
  timeout: 20000,
}, async t => {
  const directory = mkdtempSync('/tmp/apparent-dev-cargo-overlap-');
  t.after(() => rmSync(directory, {recursive: true, force: true}));
  writeFileSync(join(directory, 'Cargo.toml'), '[package]\nname="overlap-proof"\nversion="0.0.0"\nedition="2021"\n[[bin]]\nname="overlap-proof"\npath="main.rs"\n');
  writeFileSync(join(directory, 'main.rs'), String.raw`
fn main(){println!("qualified executable");}
#[test] fn build_completes_during_this_test(){
  let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
  let deadline=std::time::Instant::now()+std::time::Duration::from_secs(12);
  while !root.join("app-built").exists(){
    assert!(std::time::Instant::now()<deadline,"Cargo build remained blocked by running tests");
    std::thread::sleep(std::time::Duration::from_millis(5));
  }
  let output=std::process::Command::new(root.join("target/debug/overlap-proof")).output().unwrap();
  assert!(output.status.success());assert_eq!(String::from_utf8(output.stdout).unwrap(),"qualified executable\n");
}
`);
  const commands = {test: ['cargo', 'test', '-j', '3', '--offline', '--message-format=json-render-diagnostics', '--', '--test-threads=2'],
    build: ['node', '-e', `const {spawnSync}=require('node:child_process');const r=spawnSync('cargo',['build','-j','1','--offline'],{stdio:'inherit'});if(r.status===0)require('node:fs').writeFileSync('app-built','done');process.exit(r.status??1);`]};
  const child = spawn('node', ['--experimental-strip-types', script, JSON.stringify(commands)], {cwd: directory,
    env: {...process.env, CARGO_TARGET_DIR: join(directory, 'target')}, stdio: ['ignore', 'pipe', 'pipe']});
  let log = ''; child.stdout.on('data', chunk => {log += chunk;}); child.stderr.on('data', chunk => {log += chunk;});
  const done = once(child, 'close');
  t.after(async () => {if (child.exitCode === null && child.signalCode === null) {child.kill('SIGTERM');await done;}});
  const [code] = await done;
  assert.equal(code, 0, log); assert.match(log, /1 passed/); assert.ok(existsSync(join(directory, 'app-built')));
});
