/** The pen capability is a separate typed facade; it exposes no arbitrary native RPC. */
export { createNativePenPort } from './client.ts';
export type { NativePenPort, NativePenResult, NativePenSnapshot } from '@apparent/app-contracts/host';
