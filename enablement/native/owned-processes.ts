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

// Retain the process-group leader while the installed GUI and helpers run.
// TERM interrupts read with status >128, not EOF. Keep stdin open until the
// anchored group stop; EOF also stops this owned group if the driver dies.
const pidFileSupervisor = `
set +e
set +m
trap ':' TERM
pid_file=$1
shift
if [ "$pid_file" != "\${TMPDIR%/}/rust.pid" ] || [ ! -f "$pid_file" ] || [ -L "$pid_file" ] || [ ! -O "$pid_file" ]; then
  exit 64
fi
"$@" &
printf '%s\\n' "$!" > "$pid_file"
while :; do
  IFS= read -r line
  status=$?
  if [ "$status" -eq 1 ]; then
    kill -TERM -- -$$ 2>/dev/null || true
    sleep 0.2
    kill -KILL -- -$$
  fi
done
`;

async function stopOwned(child: import('node:child_process').ChildProcess | undefined, label: string, group = false) {
  if (!child) return { started: false };
  if (!child.pid) return { started: false, spawnFailed: true };
  const result = { pid: child.pid, group, exitCode: child.exitCode, signalCode: child.signalCode };
  const exited = () => child.exitCode !== null || child.signalCode !== null;
  const groupAbsent = () => {
    if (!group) return true;
    try { process.kill(-child.pid, 0); return false; }
    catch (error) { if (error.code === 'ESRCH') return true; throw error; }
  };
  const wait = async (ms) => {
    const until = Date.now() + ms;
    while ((!exited() || !groupAbsent()) && Date.now() < until) await new Promise(resolve => setTimeout(resolve, 50));
    return exited() && groupAbsent();
  };
  const signal = (value) => {
    assert.ok(!exited(), `${label}: leader exited; refusing an unanchored signal`);
    try {
      if (group) process.kill(-child.pid, value); else child.kill(value);
      result[value] = true;
    } catch (error) {
      if (error.code !== 'ESRCH') throw error;
      result[value] = 'already absent';
    }
  };
  if (exited()) {
    assert.ok(groupAbsent(), `${label}: leader exited with a surviving group; refusing unanchored cleanup`);
    return { ...result, exited: true, groupAbsent: group ? true : undefined };
  }
  // Stop descendants while the supervisor is still the live group anchor.
  // Reap it last; never signal a group after its leader exits.
  signal('SIGTERM');
  if (!await wait(2000)) { signal('SIGKILL'); await wait(1500); }
  Object.assign(result, { exitCode: child.exitCode, signalCode: child.signalCode,
    exited: exited(), groupAbsent: group ? groupAbsent() : undefined });
  assert.ok(result.exited && groupAbsent(), `${label} did not exit completely; owned PID/group ${child.pid}`);
  return result;
}

module.exports = { within, stopElectron, supervisor, stopBackend, pidFileSupervisor, stopOwned };
