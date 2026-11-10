/** Agent setup policy. The Python adapter alone mutates the cache using dir-fds. */
import { spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { homedir } from 'node:os';
import { resolve, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export type State = { assets: boolean; worker: boolean };
export type Status = 'unavailable' | 'downloading' | 'building' | 'ready' | 'failed';
export function plan(state: State, build: boolean, rebuild = false): string[] {
  if (!state.worker && !build) throw new Error('No published narrator worker is available. Install the source-build prerequisites, then explicitly use --build-from-source. Existing workers are reused.');
  return [...(!state.assets ? ['assets'] : []), ...(!state.worker || rebuild ? ['build'] : [])];
}
export function validWav(data: Buffer): boolean {
  // Native worker emits canonical PCM16 mono 24kHz WAV; reject truncated output.
  return data.length > 44 && data.toString('ascii', 0, 4) === 'RIFF' &&
    data.readUInt32LE(4) + 8 === data.length && data.toString('ascii', 8, 12) === 'WAVE' &&
    data.toString('ascii', 12, 16) === 'fmt ' && data.readUInt32LE(16) === 16 &&
    data.readUInt32LE(28) === 48000 && data.readUInt16LE(32) === 2 &&
    data.readUInt16LE(20) === 1 && data.readUInt16LE(22) === 1 &&
    data.readUInt32LE(24) === 24000 && data.readUInt16LE(34) === 16 &&
    data.toString('ascii', 36, 40) === 'data' && data.readUInt32LE(40) + 44 === data.length && data.length % 2 === 0 &&
    data.subarray(44).some(byte => byte !== 0);
}
function main() {
  let cache = join(homedir(), '.local/share/apparent/narrator');
  let check = false, build = false, rebuild = false;
  const args = process.argv.slice(2);
  for (let i = 0; i < args.length; i++) {
    if (args[i] === '--cache' && args[i + 1]) cache = resolve(args[++i]);
    else if (args[i] === '--check') check = true;
    else if (args[i] === '--build-from-source') build = true;
    else if (args[i] === '--rebuild-from-source') { build = true; rebuild = true; }
    else throw new Error('Usage: node capture/agent/scripts/provision-narrator.ts [--cache DIR] [--check | --build-from-source | --rebuild-from-source]');
  }
  if (check && build) throw new Error('--check cannot be combined with source-build options');
  const emit = (status: Status, detail: string) => console.log(JSON.stringify({ status, detail }));
  const adapter = fileURLToPath(new URL('./provision-narrator.py', import.meta.url));
  const call = (operation: string) => {
    const result = spawnSync('python3', [adapter, '--cache', cache, '--operation', operation],
      { encoding: 'utf8', timeout: 1810000, maxBuffer: 1024 * 1024 });
    if (result.error || result.status !== 0) throw new Error(result.error?.message || result.stderr.trim() || `${operation} failed`);
    return result.stdout;
  };
  try {
    const state: State = JSON.parse(call('check'));
    if (check && (!state.assets || !state.worker)) {
      emit('unavailable', 'Pinned assets or executable worker missing. Run setup; source build requires --build-from-source.');
      process.exitCode = 1;
      return;
    }
    if (!check) for (const step of plan(state, build, rebuild)) {
      emit(step === 'assets' ? 'downloading' : 'building', step === 'assets' ? 'Verifying/downloading pinned Kokoro model and bm_fable voice.' : 'Building native worker from the locked source manifest.');
      call(step);
    }
    const verified: State = JSON.parse(call('check'));
    if (!verified.assets || !verified.worker) throw new Error('Setup incomplete; pinned assets and executable worker are required.');
    const temporary = mkdtempSync('/tmp/apparent-dev-narrator-smoke-');
    try {
      const text = join(temporary, 'cue.txt'), output = join(temporary, 'smoke.wav');
      writeFileSync(text, 'The local narrator is ready.', { mode: 0o600 });
      const result = spawnSync(join(cache, 'apparent-narrator'), [cache, text, output], { encoding: 'utf8', timeout: 120000, maxBuffer: 1024 * 1024 });
      if (result.error || result.status !== 0) throw new Error(result.error?.message || result.stderr.trim() || 'Synthesis failed');
      if (!validWav(readFileSync(output))) throw new Error('Synthesis returned an invalid or incomplete WAV');
    } finally { rmSync(temporary, { recursive: true, force: true }); }
    emit('ready', 'Pinned assets verified and synthetic speech generated as mono PCM16 24kHz WAV. Listening quality remains unreviewed.');
  } catch (error) {
    emit('failed', error instanceof Error ? error.message : String(error));
    process.exitCode = 1;
  }
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { main(); } catch (error) {
    console.log(JSON.stringify({ status: 'failed', detail: error instanceof Error ? error.message : String(error) }));
    process.exitCode = 1;
  }
}
