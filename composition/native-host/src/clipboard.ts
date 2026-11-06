/** The Record clipboard capability is separate from the generic host and pen facades. */
export { createNativeClipboardPort } from './client.ts';
export type { NativeClipboardPort, NativeClipboardWriteResult } from '@apparent/app-contracts/host';
export { MAX_SERIALIZED_FRAME_BYTES, MAX_UTF8_TEXT_BYTES } from '@apparent/app-contracts/host';
