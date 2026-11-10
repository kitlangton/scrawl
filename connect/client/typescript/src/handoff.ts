import { createInterface } from 'node:readline';
import { pathToFileURL } from 'node:url';

export const alternatives = `Connect access is limited to 20 admitted accounts. Google sign-in alone does not grant upload or read access; waitlisted accounts cannot approve devices. Existing admitted accounts keep access when all places are occupied.

Recording still works. Keep the complete local .apparent folder.
On the recording computer: apparent inspect '/absolute/path/capture.apparent'
On another computer: use an existing authorized SSH/SCP connection or another approved file-transfer route to copy the complete folder, then inspect the destination path:
  scp -r recording-host:'/absolute/path/capture.apparent' ./capture.apparent
  apparent inspect ./capture.apparent
Replace recording-host and the example path with the actual authorized host and bundle. SSH is optional. If no approved route exists, ask the person which computer/destination to authorize; do not enable remote access, change credentials or forward private captures without permission.

For a failed Connect upload, run apparent connect status on the recording computer to find its local bundle and error. Network failures may be retried with apparent connect retry; beta_required means account access is required, not another Google login. A remote agent cannot recover the original local path from a Connect ID alone. Ask the sender for the local handoff if necessary.
After a failed upload, copy/paste the capture again to obtain its local handoff. Previously pasted Connect IDs do not change. Pending uploads can still use Connect IDs; an ID is not proof of readiness.`;

// Native code supplies only a current, consented, generation-checked queue record.
// Keep workflow eligibility here; unknown and terminal states use the local bundle.
export function handoffPointer(value: unknown): string | null {
  if (!value || typeof value !== 'object') return null;
  const { id, status } = value as Record<string, unknown>;
  return typeof id === 'string' && id.length === 32 && /^[a-f0-9]{32}$/.test(id)
    && (status === 'pending' || status === 'ready')
    ? `apparent inspect ${id}\n` : null;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
  for await (const line of lines) {
    const request = JSON.parse(line);
    process.stdout.write(`${JSON.stringify(request.operation === 'alternatives' ? alternatives : handoffPointer(request.job))}\n`);
    break;
  }
  lines.close();
}
