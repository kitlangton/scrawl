// One runtime, separate surface entrypoints and protocols.
import { cp, lstat, mkdir, realpath, rm, stat } from 'node:fs/promises';
import path from 'node:path';
async function canonicalTarget(target) {
  try { return await realpath(target); }
  catch (error) {
    if (error.code !== 'ENOENT') throw error;
    return path.join(await canonicalTarget(path.dirname(target)), path.basename(target));
  }
}
function contains(parent, child) {
  const relative = path.relative(parent, child);
  return relative === '' || (relative !== '..' && !relative.startsWith('..' + path.sep) && !path.isAbsolute(relative));
}
export async function stagePopupAssets(root, destination) {
  if (!path.isAbsolute(destination)) throw new Error('Expected absolute popup staging destination');
  const source = await realpath(root), target = await canonicalTarget(destination);
  if (contains(source, target) || contains(target, source)) throw new Error('Source and popup destination overlap');
  const previous = await lstat(destination).catch(error => {
    if (error.code !== 'ENOENT') throw error;
  });
  if (previous && (!previous.isDirectory() || previous.isSymbolicLink())) throw new Error('Unsafe popup staging destination');
  await stat(path.join(root, 'dist/index.html'));
  await stat(path.join(root, 'electron/notes/main.cjs'));
  await rm(destination, { recursive: true, force: true });
  await stagePopupEntries(root, destination);
  await cp(path.join(root, 'dist'), path.join(destination, 'dist'), { recursive: true });
  await stat(path.join(destination, 'dist/index.html'));
}
export async function stagePopupEntries(root, destination) {
  await mkdir(path.join(destination, 'electron'), { recursive: true });
  await cp(path.join(root, 'electron/runtime.cjs'), path.join(destination, 'electron/runtime.cjs'));
  for (const surface of ['notes', 'share']) {
    const source = path.join(root, 'electron', surface);
    try { await stat(source); }
    catch (error) { if (surface === 'share' && error.code === 'ENOENT') continue; throw error; }
    await stat(path.join(source, 'main.cjs'));
    await cp(source, path.join(destination, 'electron', surface), { recursive: true, force: true });
  }
}
