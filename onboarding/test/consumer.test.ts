import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';

test('packed factory declarations resolve using public packages without sibling sources', () => {
  const directory = mkdtempSync(join(tmpdir(), 'onboarding-types-'));
  const owner = resolve(import.meta.dirname, '..');
  try {
    const packed = JSON.parse(execFileSync('npm', ['pack', '--ignore-scripts', '--json', '--pack-destination', directory], { cwd: owner, encoding: 'utf8' }))[0].filename;
    const react = JSON.parse(execFileSync('npm', ['pack', '--ignore-scripts', '--json', '--pack-destination', directory, './node_modules/react'], { cwd: owner, encoding: 'utf8' }))[0].filename;
    const reactTypes = JSON.parse(execFileSync('npm', ['pack', '--ignore-scripts', '--json', '--pack-destination', directory, './node_modules/@types/react'], { cwd: owner, encoding: 'utf8' }))[0].filename;
    const csstype = JSON.parse(execFileSync('npm', ['pack', '--ignore-scripts', '--json', '--pack-destination', directory, './node_modules/csstype'], { cwd: owner, encoding: 'utf8' }))[0].filename;
    writeFileSync(join(directory, 'package.json'), JSON.stringify({ private: true, type: 'module' }));
    execFileSync('npm', ['install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund', join(directory, packed), join(owner, 'vendor/apparent-app-contracts-0.1.0.tgz'), join(owner, 'vendor/record-0.1.0-a2e7628.tgz'), join(directory, react), join(directory, reactTypes), join(directory, csstype)], { cwd: directory, stdio: 'pipe' });
    writeFileSync(join(directory, 'consumer.ts'), `import { onboardingContributionFactory, onboardingProgressCodec, adoptOnboardingProgress, type OnboardingAdoptionResult, type OnboardingFactoryDependencies, type OnboardingProgress } from '@apparent/onboarding';
import type { ContributionFactory, ProgressCodec, LegacyAdoptionBackend } from '@apparent/app-contracts/host';
const factory: ContributionFactory<OnboardingFactoryDependencies> = onboardingContributionFactory;
const codec: ProgressCodec<OnboardingProgress> = onboardingProgressCodec;
const bootstrap: (backend: LegacyAdoptionBackend) => Promise<OnboardingAdoptionResult> = adoptOnboardingProgress;
void factory; void codec; void bootstrap;
async function ownerRepair(dependencies: OnboardingFactoryDependencies) {
  const concrete = await onboardingContributionFactory.create(dependencies);
  await concrete.activation.permissionRepair.activate(['microphone']);
  return concrete.module;
}
void ownerRepair;
`);
    execFileSync(process.execPath, [join(owner, 'node_modules/typescript/bin/tsc'), '--noEmit', '--strict', '--module', 'nodenext', '--moduleResolution', 'nodenext', '--target', 'es2022', '--lib', 'es2022,dom', 'consumer.ts'], { cwd: directory, stdio: 'pipe' });
    const result = execFileSync(process.execPath, ['--input-type=module', '-e', "import {onboardingContributionFactory,onboardingProgressCodec} from '@apparent/onboarding'; console.log(onboardingContributionFactory.ownerId,onboardingProgressCodec.decode(null).stage)"], { cwd: directory, encoding: 'utf8' });
    assert.equal(result.trim(), 'onboarding welcome');
  } finally { rmSync(directory, { recursive: true, force: true }); }
});
