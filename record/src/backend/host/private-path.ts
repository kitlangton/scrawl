import { constants } from "node:fs";
import { lstat, mkdir, open, readFile, readdir, realpath } from "node:fs/promises";
import { isAbsolute, join, parse, resolve } from "node:path";

const privateMode = 0o700;

export function hostError(): Error {
  return new Error("Record host storage is unavailable or invalid");
}

export async function ensurePrivateRoot(root: string): Promise<string> {
  if (typeof root !== "string" || !isAbsolute(root) || resolve(root) !== root || root.includes("\0")) throw hostError();
  const { root: volume } = parse(root);
  if (root === volume) throw hostError();
  let current = volume;
  const parts = root.slice(volume.length).split(/[\\/]+/u).filter(Boolean);
  for (const part of parts) {
    current = join(current, part);
    try {
      await mkdir(current, { mode: privateMode });
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw hostError();
    }
    const info = await lstat(current).catch(() => { throw hostError(); });
    if (!info.isDirectory() || info.isSymbolicLink()) throw hostError();
    if (current === root && (info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0)) throw hostError();
  }
  if (await realpath(root).catch(() => "") !== root) throw hostError();
  return root;
}

export async function ensurePrivateFile(path: string): Promise<void> {
  try {
    const handle = await open(path, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | (constants.O_NOFOLLOW ?? 0), 0o600);
    await handle.close();
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw hostError();
  }
  const info = await lstat(path).catch(() => { throw hostError(); });
  if (!info.isFile() || info.isSymbolicLink() || info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0) throw hostError();
}

export async function privateMarker(path: string): Promise<string> {
  const info = await lstat(path).catch(() => { throw hostError(); });
  if (!info.isFile() || info.isSymbolicLink() || info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0) throw hostError();
  return readFile(path, "utf8").catch(() => { throw hostError(); });
}

export function reservationMarker(root: string, name: string): string {
  return join(root, name, ".record-reservation");
}

export async function ensurePrivateReservation(path: string, root: string, requestId: string): Promise<void> {
  const name = path.slice(root.length + 1);
  if (!name || name.includes("/") || name.includes("\\")) throw hostError();
  try {
    await mkdir(path, { mode: privateMode });
    const marker = reservationMarker(root, name);
    const handle = await open(marker, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | (constants.O_NOFOLLOW ?? 0), 0o600);
    try { await handle.writeFile(requestId, "utf8"); } finally { await handle.close(); }
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw hostError();
    const info = await lstat(path).catch(() => { throw hostError(); });
    if (!info.isDirectory() || info.isSymbolicLink() || info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0) throw hostError();
    if ((await readdir(path).catch(() => { throw hostError(); })).some(entry => entry !== ".record-reservation")) throw hostError();
    const marker = reservationMarker(root, name);
    let matched = false;
    for (let attempt = 0; attempt < 100; attempt++) {
      try {
        if (await privateMarker(marker) === requestId) { matched = true; break; }
      } catch { /* another process may still be writing the reservation marker */ }
      await new Promise(resolve => setTimeout(resolve, 10));
    }
    if (!matched) throw hostError();
  }
  const reservation = await lstat(path).catch(() => { throw hostError(); });
  if (!reservation.isDirectory() || reservation.isSymbolicLink() || reservation.uid !== process.getuid?.() || (reservation.mode & 0o077) !== 0) throw hostError();
  if (await privateMarker(reservationMarker(root, name)) !== requestId) throw hostError();
}
