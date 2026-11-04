import { createHash } from 'node:crypto';
import { targets } from './seal.ts';

/** A reviewed release pin, never inferred from an untrusted manifest. */
export interface Pin { sourceSha: string; run: string; attempt: string; target: string; sha256: string }
export function validate(bytes: Buffer, manifest: Record<string, unknown>, pin: Pin) {
  if (!targets.includes(pin.target) || !/^[a-f0-9]{40}$/.test(pin.sourceSha) ||
      !/^[1-9][0-9]*$/.test(pin.run) || !/^[1-9][0-9]*$/.test(pin.attempt) ||
      !/^[a-f0-9]{64}$/.test(pin.sha256)) throw Error('Invalid release pin');
  const expected = { schema: 1, protocol: 1, repository: 'tryapparent/apparent',
    workflow: '.github/workflows/connect-delivery.yml', executable: 'apparent-connect-recipient',
    ...pin, version: `recipient-${pin.sourceSha}-${pin.run}-${pin.attempt}` };
  for (const [key, value] of Object.entries(expected)) {
    if (manifest[key] !== value) throw Error(`Incompatible recipient ${key}`);
  }
  if (createHash('sha256').update(bytes).digest('hex') !== pin.sha256) throw Error('Recipient digest mismatch');
  const arm = pin.target.startsWith('aarch64');
  if (pin.target.endsWith('linux-gnu')) {
    if (bytes.length < 64 || bytes.subarray(0,4).toString('hex') !== '7f454c46' ||
        bytes[4] !== 2 || bytes[5] !== 1 || bytes.readUInt16LE(18) !== (arm ? 183 : 62) ||
        ![2,3].includes(bytes.readUInt16LE(16))) throw Error('Invalid ELF executable architecture');
  } else if (bytes.length < 32 || bytes.readUInt32LE(0) !== 0xfeedfacf ||
      bytes.readUInt32LE(4) !== (arm ? 0x100000c : 0x1000007) || bytes.readUInt32LE(12) !== 2) {
    throw Error('Invalid Mach-O executable architecture');
  }
  return expected;
}
