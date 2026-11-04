export const STORAGE_BRIDGE_TEST = 'capture::desktop::workflow::agent_capture::tests::storage_electron_rust_roundtrip';

export function verifyBridgeListing(testName: unknown, code: number, output: string): void {
  if (testName !== STORAGE_BRIDGE_TEST || code !== 0 ||
      !output.split(/\r?\n/).includes(`${STORAGE_BRIDGE_TEST}: test`)) {
    throw new Error('Prepared backend does not list the required Storage bridge test');
  }
}

export function verifyBridgeResult(code: number, output: string): void {
  if (code !== 0 || !output.split(/\r?\n/).includes(`test ${STORAGE_BRIDGE_TEST} ... ok`) ||
      !/^test result: ok\. 1 passed; 0 failed; 0 ignored;/m.test(output)) {
    throw new Error('Prepared backend did not execute and pass the required Storage bridge test');
  }
}
