// Native command output can include private paths. Retain only fixed protocol enums.
export function captureFailureReceipt(operation: string, stdout: string) {
  let receipt: Record<string, unknown> = {};
  try { receipt = JSON.parse(stdout); } catch {}
  if (!receipt || typeof receipt !== 'object' || Array.isArray(receipt)) receipt = {};
  return {
    operation: ['start', 'stop', 'inspect', 'cancel'].includes(operation) ? operation : 'unknown',
    status: typeof receipt.status === 'string' && ['starting', 'recording', 'processing', 'saved', 'failed', 'cancelled'].includes(receipt.status) ? receipt.status : 'unknown',
    error: receipt.error == null ? null : typeof receipt.error === 'string' && ['start_failed', 'finalization_failed', 'timeout', 'control_unavailable', 'app_busy', 'app_unresponsive', 'invalid_response', 'cancelled', 'unknown_capture', 'ended_before_ready', 'occupied', 'admission_expired'].includes(receipt.error) ? receipt.error : 'unknown',
  };
}
