// Owned native-test child lifecycle. Callers own admission and private evidence.
const assert = require('node:assert/strict');
const { execFile } = require('node:child_process');
const { promisify } = require('node:util');
const run = promisify(execFile);

async function within<T>(operation: Promise<T>, ms: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([operation, new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error('Packaged smoke operation timed out')), ms);
    })]);
  } finally { clearTimeout(timer); }
}

async function stopElectron(application: { close(): Promise<unknown>; process(): import('node:child_process').ChildProcess } | undefined, timeout = 3000) {
  if (!application) return;
  try { await within(application.close(), timeout); }
  catch (error) {
    const child = application.process();
    if (child.exitCode === null && child.signalCode === null) {
      const exited = new Promise(resolve => child.once('exit', resolve));
      child.kill('SIGKILL');
      await within(exited, 1500);
    }
    throw error; // Forced shutdown is a failed smoke check, never a pass.
  }
}

// Retain a live process-group anchor until teardown. EOF also kills this group
// if the test driver dies; never look up or kill another installation by name.
const supervisor = `
set +e
set +m
trap ':' TERM
"$@" &
while :; do
  IFS= read -r line
  if [ "$?" -eq 1 ]; then
    kill -TERM -- -$$ 2>/dev/null || true
    sleep 0.2
    kill -KILL -- -$$
  fi
done
`;

async function stopBackend(child: import('node:child_process').ChildProcess | undefined) {
  if (!child?.pid) return;
  const exited = () => child.exitCode !== null || child.signalCode !== null;
  const absent = async () => {
    try { process.kill(-child.pid, 0); return false; }
    catch (error) {
      if (error.code === 'ESRCH') return true;
      // XNU may report EPERM for a group whose only remaining members are
      // zombies. Never infer that from the exited supervisor alone.
      if (error.code !== 'EPERM' || process.platform !== 'darwin' || !exited()) throw error;
      const { stdout } = await run('/bin/ps', ['-axo', 'pgid=,stat='],
        { timeout: 1000, maxBuffer: 1024 * 1024 });
      assert.ok(stdout.trim(), 'Empty backend process-group snapshot; fixture retained');
      for (const line of stdout.split('\n').filter(line => line.trim())) {
        const fields = line.trim().split(/\s+/);
        assert.ok(fields.length === 2 && /^\d+$/.test(fields[0]) && /^[A-Za-z]/.test(fields[1]),
          'Invalid backend process-group snapshot; fixture retained');
        if (Number(fields[0]) === child.pid && !fields[1].startsWith('Z')) throw error;
      }
      return true; // No live members; do not signal the old group ID again.
    }
  };
  if (!exited()) {
    process.kill(-child.pid, 'SIGTERM');
    await new Promise(resolve => setTimeout(resolve, 200));
    // The supervisor ignores TERM; retain its identity until the final signal.
    if (!exited()) process.kill(-child.pid, 'SIGKILL');
  }
  for (let n = 0; n < 40; n++) {
    if (exited() && await absent()) return;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  assert.ok(exited() && await absent(), 'Owned backend group did not exit; fixture retained');
}

module.exports = { within, stopElectron, supervisor, stopBackend };
