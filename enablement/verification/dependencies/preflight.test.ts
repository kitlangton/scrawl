import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import test from 'node:test';

const verification = resolve('enablement/verification.py');

function pythonFixture(body: string): void {
  const result = spawnSync('python3', ['-c', body, verification], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr || result.stdout);
}

test('preflight refuses a missing selected npm install before PR contract commands', () => {
  pythonFixture(`
import importlib.util, json, pathlib, sys, tempfile
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('verification',sys.argv[1]); v=importlib.util.module_from_spec(spec); spec.loader.exec_module(v)
with tempfile.TemporaryDirectory() as d:
 root=pathlib.Path(d); package=root/'composition/architecture'; package.mkdir(parents=True)
 (package/'package.json').write_text(json.dumps({'devDependencies':{'typescript':'1'}}))
 (package/'package-lock.json').write_text(json.dumps({'packages':{'':{'devDependencies':{'typescript':'1'}},'node_modules/typescript':{'version':'1'}}}))
 with patch.object(v,'ROOT',root), patch.object(v.subprocess,'run') as run:
  try: v.run_pr_checks({'paths':['composition/app/src/main.tsx'],'app_shell':False})
  except RuntimeError as error: assert 'Selected verification package composition/architecture' in str(error)
  else: raise AssertionError('missing selected install was not refused')
  assert not run.called, 'contracts ran before refusal'
`);
});

test('Public preflight reuses build prerequisites; unrelated docs stay skipped', () => {
  pythonFixture(`
import importlib.util, json, pathlib, sys, tempfile
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('verification',sys.argv[1]); v=importlib.util.module_from_spec(spec); spec.loader.exec_module(v)
with tempfile.TemporaryDirectory() as d:
 root=pathlib.Path(d); package=root/'public/site'; package.mkdir(parents=True)
 for name in ('package.json','bun.lock','tsconfig.json','vite.config.ts','index.html'): (package/name).write_text('{}')
 (package/'package.json').write_text(json.dumps({'dependencies':{'vite':'1'}}))
 with patch.object(v,'ROOT',root):
  with patch.object(v,'check_public_site_prerequisites',side_effect=RuntimeError('public prereq')) as check:
   try: v.preflight_dependencies({'paths':['public/site/src/features.json']},include_architecture=True)
   except RuntimeError as error: assert str(error)=='public prereq'
   else: raise AssertionError('Public prerequisite was skipped')
   check.assert_called_once_with(package)
  v.preflight_dependencies({'paths':['enablement/docs/continuous-delivery.md']})
`);
});

test('browser changed-tree preflight checks Bun inputs without installing', () => {
  pythonFixture(`
import importlib.util, pathlib, sys, tempfile
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('verification',sys.argv[1]); v=importlib.util.module_from_spec(spec); spec.loader.exec_module(v)
with tempfile.TemporaryDirectory() as d:
 root=pathlib.Path(d); package=root/'connect/web'; package.mkdir(parents=True)
 for name in ('package.json','bun.lock'): (package/name).write_text('{}')
 with patch.object(v,'ROOT',root), patch.object(v.shutil,'which',return_value=None):
  try: v.preflight_dependencies({'paths':['connect/web/src/main.tsx'],'browser':True},include_browser=True)
  except RuntimeError as error: assert 'needs Bun on PATH' in str(error)
  else: raise AssertionError('missing Bun was not refused')
`);
});

test('onboarding renderer preflight predicate includes renderer and shared snapshot inputs', () => {
  pythonFixture(`
import importlib.util, sys
spec=importlib.util.spec_from_file_location('verification',sys.argv[1]); v=importlib.util.module_from_spec(spec); spec.loader.exec_module(v)
for path in ('capture/desktop/ui/frontend/src/onboarding/form.tsx','capture/desktop/ui/frontend/vite.config.ts','shared/ui/src/button.tsx','shared/assets/fonts/HostGrotesk-Variable.ttf'):
 assert v.onboarding_renderer_selected([path]), path
assert not v.onboarding_renderer_selected(['enablement/docs/continuous-delivery.md'])
`);
});

test('prepared architecture runs contracts; stale versions refuse and unknown paths remain conservative', () => {
  pythonFixture(`
import importlib.util, json, pathlib, sys, tempfile
from unittest.mock import patch
spec=importlib.util.spec_from_file_location('verification',sys.argv[1]); v=importlib.util.module_from_spec(spec); spec.loader.exec_module(v)
with tempfile.TemporaryDirectory() as d:
 root=pathlib.Path(d); package=root/'composition/architecture'; installed=package/'node_modules/typescript'; installed.mkdir(parents=True)
 manifest={'devDependencies':{'typescript':'1'}}
 lock={'packages':{'':manifest,'node_modules/typescript':{'version':'1'}}}
 (package/'package.json').write_text(json.dumps(manifest)); (package/'package-lock.json').write_text(json.dumps(lock))
 (package/'node_modules/.package-lock.json').write_text(json.dumps(lock)); (installed/'package.json').write_text(json.dumps({'version':'1'}))
 with patch.object(v,'ROOT',root), patch.object(v.subprocess,'run') as run:
  v.run_pr_checks(v.select(['composition/architecture/README.md']) | {'app_shell':False,'record':False,'agent':False,'onboarding_package':False,'delivery':False,'platform_resources':False})
  assert any(call.args[0][0]=='python3' for call in run.call_args_list), 'required contracts skipped'
  # npm represents directory dependencies as links with version on the target entry.
  lock['packages']['node_modules/typescript']={'resolved':'../typescript','link':True}
  lock['packages']['../typescript']={'version':'1'}
  (package/'package-lock.json').write_text(json.dumps(lock)); (package/'node_modules/.package-lock.json').write_text(json.dumps(lock))
  v.preflight_dependencies({'paths':['composition/architecture/src/cli.ts']},include_architecture=True)
  run.reset_mock(); (installed/'package.json').write_text(json.dumps({'version':'stale'}))
  try: v.run_pr_checks(v.select(['unknown-owner/file.ts']))
  except RuntimeError as error: assert 'installed package version differs' in str(error)
  else: raise AssertionError('stale unknown-path prerequisite was not refused')
  assert not run.called
`);
});
