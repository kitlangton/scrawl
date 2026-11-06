import assert from 'node:assert/strict';
import { constants } from 'node:fs';
import { chmod, lstat, mkdir, open, realpath, writeFile } from 'node:fs/promises';
import { isAbsolute, join } from 'node:path';
import { digest, exactFiles } from './artifacts.ts';
import { safeNativePath, type NativeFileModes } from './native-modes.ts';

export interface AdmittedRecordArtifact {
  /** Extracted payload from the canonical delivery consumer, never renderer input. */
  directory: string;
  /** Independently admitted manifest digest and original build source. */
  manifestSha256: string;
  sourceSha: string;
}
export interface RecordNativeArtifacts {
  target: 'linux-x64';
  capture: AdmittedRecordArtifact;
  media: AdmittedRecordArtifact;
  config: AdmittedRecordArtifact;
}

async function bytes(path: string, maximum: number): Promise<{ data: Buffer; mode: number }> {
  const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const before = await file.stat();
    assert(before.isFile() && before.nlink === 1 && before.size > 0 && before.size <= maximum
      && !(before.mode & 0o7022), 'Unsafe native artifact file');
    const value = Buffer.alloc(before.size + 1);
    let length = 0;
    while (length < value.length) {
      const read = await file.read(value, length, value.length - length, length);
      if (!read.bytesRead) break;
      length += read.bytesRead;
    }
    const after = await file.stat();
    const current = await lstat(path);
    assert(length === before.size && before.dev === after.dev && before.ino === after.ino
      && before.mtimeMs === after.mtimeMs && before.ctimeMs === after.ctimeMs && before.mode === after.mode
      && !current.isSymbolicLink() && current.dev === after.dev && current.ino === after.ino
      && current.mode === after.mode && current.ctimeMs === after.ctimeMs,
    'Native artifact changed during read');
    return { data: value.subarray(0, length), mode: before.mode & 0o7777 };
  } finally { await file.close(); }
}

/** Copies the complete closed artifacts. Admission/provenance remains the external delivery consumer's responsibility. */
export async function stageRecordNative(output: string, input: RecordNativeArtifacts): Promise<NativeFileModes> {
  assert(input?.target === 'linux-x64', 'Record Capture/2 and Media/2 assembly supports Linux x64 only');
  // Snapshot trusted configuration before any filesystem await.
  const inputs = structuredClone(input);
  const modes: NativeFileModes = {};
  const staged: Array<{ path: string; data: Buffer; mode: 0o644 | 0o755 }> = [];
  let total = 0;
  for (const slot of ['capture', 'media', 'config'] as const) {
    const artifact = inputs[slot];
    assert(artifact && isAbsolute(artifact.directory) && /^[a-f0-9]{64}$/.test(artifact.manifestSha256)
      && /^[a-f0-9]{40}$/.test(artifact.sourceSha), 'Invalid admitted Record artifact');
    assert.equal(await realpath(artifact.directory), artifact.directory, 'Native artifact directory must be canonical');
    const directory = await lstat(artifact.directory);
    assert(directory.isDirectory() && !(directory.mode & 0o7022), 'Unsafe native artifact directory');
    const manifestFile = await bytes(join(artifact.directory, 'manifest.json'), 1024 * 1024);
    const raw = manifestFile.data;
    assert.equal(digest(raw), artifact.manifestSha256, 'Native manifest differs from admitted digest');
    const manifest = JSON.parse(raw.toString());
    assert(manifest.schema === 1 && manifest.sourceSha === artifact.sourceSha && manifest.target === inputs.target,
      'Native artifact source or target mismatch');
    assert(Array.isArray(manifest.runtimeFiles) && manifest.runtimeFiles.length <= 1024, 'Invalid native runtime inventory');
    let files: Array<{ path: string; sha256: string; mode: number }> = manifest.runtimeFiles;
    if (slot === 'config') {
      assert(manifest.protocol === 1 && files.length === 0 && manifest.mode === 0o755, 'Invalid config helper contract');
      files = [{ path: 'apparent-config-host', sha256: manifest.sha256, mode: manifest.mode }];
    } else {
      const entry = slot === 'capture' ? 'apparent-engine-capture-launch' : 'apparent-engine-frames';
      assert(manifest.launch?.entry === entry && manifest.launch.pathLookup === false && manifest.launch.runtimeBuilds === false,
        'Invalid native launch contract');
      assert(manifest.capabilities?.includes(slot === 'capture' ? 'capture/2' : 'media/2'), 'Required native protocol unavailable');
      assert(files.some(file => file.path === entry && file.mode === 0o755), 'Native entry absent from inventory');
    }
    assert(new Set(files.map(file => file.path)).size === files.length, 'Duplicate native runtime path');
    for (const file of files) {
      assert(safeNativePath(file.path) && file.path !== 'manifest.json' && /^[a-f0-9]{64}$/.test(file.sha256)
        && (file.mode === 0o644 || file.mode === 0o755), 'Invalid native runtime entry');
    }
    await exactFiles(artifact.directory, ['manifest.json', ...files.map(file => file.path)]);
    for (const file of [...files, { path: 'manifest.json', sha256: artifact.manifestSha256, mode: 0o644 }]) {
      const snapshot = file.path === 'manifest.json' ? manifestFile : await bytes(join(artifact.directory, file.path), 128 * 1024 * 1024);
      const { data } = snapshot;
      assert.equal(digest(data), file.sha256, 'Native runtime checksum mismatch');
      assert.equal(snapshot.mode, file.mode, 'Native runtime mode mismatch');
      total += data.length;
      assert(total <= 512 * 1024 * 1024, 'Native Record closure exceeds assembly limit');
      staged.push({ path: `native-record/${slot}/${file.path}`, data, mode: file.mode as 0o644 | 0o755 });
    }
  }
  // Refuse an existing destination. No partial native closure is published on validation failure.
  await mkdir(join(output, 'native-record'));
  for (const file of staged) {
    const target = join(output, file.path);
    await mkdir(join(target, '..'), { recursive: true });
    await writeFile(target, file.data, { flag: 'wx', mode: file.mode });
    await chmod(target, file.mode);
    modes[file.path] = file.mode;
  }
  return modes;
}

