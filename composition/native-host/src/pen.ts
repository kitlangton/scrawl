/** The pen capability is a separate typed facade; it exposes no arbitrary native RPC. */
export { createNativePenPort } from './client.js';
export type { NativePenPort, NativePenResult, NativePenSnapshot } from '@apparent/app-contracts/native';
