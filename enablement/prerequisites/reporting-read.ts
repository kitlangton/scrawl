import assert from 'node:assert/strict';

export type ReportingReadObservation = {
  elapsedMs: number;
  durationMs: number;
  response?: unknown;
  error?: string;
  late?: boolean;
};

type ReportingRead = {
  read: () => Promise<unknown>;
  enabled: boolean;
  label: string;
  timeoutMs: number;
  deadline: number;
  observations: ReportingReadObservation[];
};

// Native-test admission only. The caller supplies a read capability, never a
// toggle/click or other mutation. Product policy and persistence stay in Rust.
// Only an explicit app_busy response permits another observation. Each request
// and backoff consumes the SAME original read budget, capped by the fixture's
// overall deadline. A timeout or wrong value must never poll itself into a pass.
export async function assertReportingRead({ read, enabled, label, timeoutMs, deadline, observations }: ReportingRead): Promise<void> {
  const started = Date.now();
  const until = Math.min(started + timeoutMs, deadline);
  const expired = () => new Error(`${label} timed out after ${until - started}ms`,
    { cause: observations[0]?.response });
  while (true) {
    if (Date.now() >= until) throw expired();
    const attempt = Date.now();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let response: unknown;
    try {
      response = await Promise.race([read(), new Promise((_, reject) => {
        timer = setTimeout(() => reject(expired()), Math.max(0, until - Date.now()));
      })]);
    } catch (error) {
      observations.push({ elapsedMs: Date.now() - started, durationMs: Date.now() - attempt,
        error: error instanceof Error ? error.message : String(error) });
      throw error;
    } finally { clearTimeout(timer); }
    const late = Date.now() >= until;
    observations.push({ elapsedMs: Date.now() - started, durationMs: Date.now() - attempt, response, late });
    if (late) throw expired();
    if (response !== null && typeof response === 'object' &&
        'status' in response && response.status === 'error' &&
        'code' in response && response.code === 'app_busy') {
      await new Promise(resolve => setTimeout(resolve, Math.min(150, Math.max(0, until - Date.now()))));
      continue;
    }
    assert.deepEqual(response, { status: 'connected', enabled }, label);
    return;
  }
}
