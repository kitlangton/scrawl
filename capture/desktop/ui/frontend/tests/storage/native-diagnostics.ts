import assert from 'node:assert/strict';
import path from 'node:path';

type WindowIdentity = { pid: number; title: string; bounds: { x: number; y: number; width: number; height: number } };
type Run = (file: string, args: string[], options: object) => Promise<unknown>;

// Cache identity while Electron still responds. A hung native dialog must not
// make failure evidence depend on another main-process IPC roundtrip.
export async function retainPickerFailure(identity: WindowIdentity, ownedPid: number,
  output: string, run: Run, budget: (cap: number) => number, env: object) {
  assert.ok(Number.isSafeInteger(ownedPid) && ownedPid > 0);
  assert.equal(identity.pid, ownedPid);
  assert.equal(identity.title, 'Apparent — UI foundation');
  const { x, y, width, height } = identity.bounds;
  assert.ok([x, y, width, height].every(Number.isSafeInteger) && width > 0 && height > 0);
  assert.ok(path.isAbsolute(output));
  const evidence: Record<string, string> = {};
  const observations: [string, string, string, string[]][] = [
    ['lastKnownWindowScreenshot', 'failed-owned-window.png', '/usr/sbin/screencapture', ['-x', '-R', [x, y, width, height].join(',')]],
    ['ownedProcessSample', 'electron-main-sample.txt', '/usr/bin/sample', [String(ownedPid), '1', '1', '-file']],
  ];
  for (const [key, file, command, args] of observations) {
    try {
      await run(command, [...args, path.join(output, file)], {
        env, timeout: budget(3000), killSignal: 'SIGKILL', maxBuffer: 8192,
      });
      evidence[key] = file;
    } catch (error) { evidence[`${key}Unavailable`] = (error as Error).message; }
  }
  return evidence;
}
