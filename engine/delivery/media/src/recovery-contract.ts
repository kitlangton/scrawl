import { isDeepStrictEqual } from 'node:util'

export type RecoveryLaunch = {
  schema: 'apparent.engine-recovery-peer/1'
  channel: 'capture/2' | 'media/2'
  argv: string[]
  startsFreshCapture: false
  onInputEof: 'retain-admitted-finite-work'
}

export function recoveryLaunch(channel: RecoveryLaunch['channel']): RecoveryLaunch {
  const argv = channel === 'capture/2'
    ? ['--runtime-directory', '<private-runtime-dir>', '--capture-channel', 'capture', '--capture-protocol-version', '2']
    : ['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '2']
  return { schema: 'apparent.engine-recovery-peer/1', channel, argv: [...argv, '--recovery-peer-v1', '<private-peer-nonce>'], startsFreshCapture: false, onInputEof: 'retain-admitted-finite-work' }
}

/** Absence preserves old artifact readers; it never implies safe release. */
export function validateRecoveryAdvertisement(value: unknown, channel: RecoveryLaunch['channel'], supported = true): void {
  if (value !== undefined && (!supported || !isDeepStrictEqual(value, recoveryLaunch(channel)))) throw new Error('unsupported recovery lifetime launch identity')
}
