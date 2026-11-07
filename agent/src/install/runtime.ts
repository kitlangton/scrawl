import { execFileSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { MediaClient, launchMediaWorker } from '@apparent/engine-sdk/media';
import { CaptureClient, launchCaptureEngine } from '@apparent/engine-sdk/capture';
import type { HandshakeResult, InstallerDependencies } from './types.js';

export const systemInstallerDependencies: InstallerDependencies = {
  installNpm(snapshotPath, installRoot, privateHome) {
    const invocation = makeNpmInvocation(snapshotPath, installRoot, privateHome,
      join(dirname(installRoot), 'npm-cache'), process.env.PATH ?? '/usr/bin:/bin', join(dirname(installRoot), 'npm-tmp'));
    execFileSync('/bin/sh', ['-c', 'umask 077; exec "$@"', 'apparent-installer-npm', 'npm', ...invocation.args], {
      cwd: invocation.cwd,
      env: invocation.env,
      stdio: 'pipe',
      timeout: 120_000,
    });
  },

  smokeCli(cliPath, privateHome, installRoot) {
    const output = execFileSync(process.execPath, [cliPath, '--version'], {
      cwd: installRoot,
      env: { PATH: '/usr/bin:/bin', HOME: privateHome },
      stdio: 'pipe',
      timeout: 15_000,
      encoding: 'utf8',
    });
    if (output.trim() !== '0.1.0') throw new Error('installed Agent CLI version smoke check failed');
  },

  async handshake(_sdkMediaModule, entry, runtime): Promise<HandshakeResult> {
    // Use the installer's published SDK dependency; core separately validates
    // the installed consumer's matching SDK version and media export.
    const client = new MediaClient(launchMediaWorker(entry, runtime, 3000));
    let reply: HandshakeResult['reply'] | undefined;
    let failure: unknown;
    try {
      reply = await client.handshake();
    } catch (error) {
      failure = error;
    }
    let closeOutcome: string;
    try {
      closeOutcome = await client.close();
    } catch (error) {
      throw new Error(`native worker close failed: ${(error as Error).message}`);
    }
    if (failure) throw new Error(`native media handshake failed: ${(failure as Error).message}`);
    if (!reply) throw new Error('native media handshake returned no response');
    return { reply, closeOutcome };
  },

  async captureHandshake(entry, runtime, shutdownTimeoutMs) {
    const client = new CaptureClient(launchCaptureEngine(entry, runtime, shutdownTimeoutMs));
    let reply: Awaited<ReturnType<typeof client.handshake>> | undefined;
    let failure: unknown;
    try { reply = await client.handshake(); } catch (error) { failure = error; }
    let closeOutcome: string;
    try { closeOutcome = await client.close(); } catch (error) {
      throw new Error(`native capture worker close failed: ${(error as Error).message}`);
    }
    if (failure) throw new Error(`native capture handshake failed: ${(failure as Error).message}`);
    if (!reply) throw new Error('native capture handshake returned no response');
    return { reply, closeOutcome };
  },
};

export type NpmInvocation = {
  args: string[];
  cwd: string;
  env: Record<string, string>;
  umask: number;
};

export function makeNpmInvocation(
  snapshotPath: string,
  installRoot: string,
  privateHome: string,
  privateCache: string,
  path: string,
  privateTemp = join(dirname(installRoot), 'npm-tmp'),
): NpmInvocation {
  return {
    args: ['install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund', '--umask=077', '--prefix', installRoot, snapshotPath],
    cwd: installRoot,
    umask: 0o077,
    env: {
      PATH: path,
      HOME: privateHome,
      TMPDIR: privateTemp,
      npm_config_offline: 'true',
      npm_config_ignore_scripts: 'true',
      npm_config_userconfig: join(privateHome, 'user.npmrc'),
      npm_config_globalconfig: join(privateHome, 'global.npmrc'),
      npm_config_cache: privateCache,
      npm_config_prefix: installRoot,
      npm_config_audit: 'false',
      npm_config_fund: 'false',
      npm_config_update_notifier: 'false',
    },
  };
}
