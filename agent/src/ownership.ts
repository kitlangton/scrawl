import { constants } from 'node:fs';
import { lstat, mkdir, open } from 'node:fs/promises';
import { join } from 'node:path';
import type { StartRequest } from '@apparent/engine-sdk';

/** Private recovery journal. This is caller authority, never bundle/public output. */
export async function retainRequest(runtime: string, request: StartRequest): Promise<string> {
  const directory = join(runtime, 'agent-requests');
  const checkDirectory = async (path: string) => {
    const info = await lstat(path);
    if (!info.isDirectory() || info.isSymbolicLink() || (info.mode & 0o077) !== 0 || info.uid !== process.getuid!())
      throw new Error('Engine runtime and Agent recovery directory must be private and owned by the current user');
  };
  await checkDirectory(runtime);
  await mkdir(directory, { mode: 0o700 }).catch((error: NodeJS.ErrnoException) => { if (error.code !== 'EEXIST') throw error; });
  await checkDirectory(directory);
  const path = join(directory, `${request.captureId}.json`);
  const file = await open(path, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600);
  try { await file.writeFile(JSON.stringify(request)); await file.sync(); } finally { await file.close(); }
  const parent = await open(directory, constants.O_RDONLY);
  try { await parent.sync(); } finally { await parent.close(); }
  return path;
}

export async function loadRequest(runtime: string, captureId: string): Promise<StartRequest> {
  if (!/^[0-9a-f]{32}$/.test(captureId)) throw new Error('Expected a 32-character capture ID');
  for (const directory of [runtime, join(runtime, 'agent-requests')]) {
    const info = await lstat(directory);
    if (!info.isDirectory() || info.isSymbolicLink() || (info.mode & 0o077) || info.uid !== process.getuid!())
      throw new Error('Recovery directory must be private and owned by the current user');
  }
  const file = await open(join(runtime, 'agent-requests', `${captureId}.json`), constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  try {
    const info = await file.stat();
    if (!info.isFile() || info.size > 16384 || (info.mode & 0o077) || info.uid !== process.getuid!()) throw new Error('Invalid private recovery record');
    const buffer = Buffer.alloc(16385);
    const {bytesRead} = await file.read(buffer, 0, buffer.length, 0);
    if (bytesRead > 16384) throw new Error('Invalid private recovery record');
    const request = JSON.parse(new TextDecoder('utf-8', {fatal:true}).decode(buffer.subarray(0, bytesRead)));
    if (request.captureId !== captureId || !/^[0-9a-f]{64}$/.test(request.ownerToken)) throw new Error('Invalid private recovery record');
    return request as StartRequest;
  } catch { throw new Error('Cannot read valid private recovery record'); }
  finally { await file.close(); }
}
