import { chmod, mkdir, mkdtemp, realpath, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { digest } from '../../scripts/artifacts.ts';
import type { RecordNativeArtifacts } from '../../scripts/record-native.ts';

export async function fixture() {
  const work = await realpath(await mkdtemp(join(tmpdir(), 'record-native-closure-')));
  const sourceSha = 'a'.repeat(40);
  const input: RecordNativeArtifacts = { target: 'linux-x64' } as RecordNativeArtifacts;
  for (const slot of ['capture', 'media', 'config'] as const) {
    const directory = join(work, slot);
    await mkdir(directory, { mode: 0o700 });
    const entry = slot === 'capture' ? 'apparent-engine-capture-launch' : slot === 'media' ? 'apparent-engine-frames' : 'apparent-config-host';
    const data = Buffer.from('#!/bin/sh\nexit 0\n');
    await writeFile(join(directory, entry), data, { mode: 0o755 });
    await chmod(join(directory, entry), 0o755);
    const runtimeFiles = [{ path: entry, mode: 0o755, sha256: digest(data) }];
    if (slot !== 'config') {
      await mkdir(join(directory, 'libexec'));
      await writeFile(join(directory, 'libexec/worker'), data, { mode: 0o755 });
      await chmod(join(directory, 'libexec/worker'), 0o755);
      runtimeFiles.push({ path: 'libexec/worker', mode: 0o755, sha256: digest(data) });
    }
    const manifest = { schema: 1, sourceSha, target: 'linux-x64',
      ...(slot === 'config' ? { protocol: 1, sha256: digest(data), mode: 0o755, runtimeFiles: [] }
        : { launch: { entry, pathLookup: false, runtimeBuilds: false }, runtimeFiles,
          capabilities: [slot === 'capture' ? 'capture/2' : 'media/2'] }) };
    const raw = Buffer.from(JSON.stringify(manifest));
    await writeFile(join(directory, 'manifest.json'), raw, { mode: 0o644 });
    await chmod(join(directory, 'manifest.json'), 0o644);
    input[slot] = { directory, sourceSha, manifestSha256: digest(raw) };
  }
  const output = join(work, 'shell'); await mkdir(output);
  return { work, output, input };
}

