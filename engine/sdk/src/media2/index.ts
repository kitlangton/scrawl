/** Frozen derivative documents plus explicitly negotiated client/transport.
 * Native authority and runtime discovery require separate producer evidence. */
export type * from "./types.ts";
export { prepareMedia2Request, encodeContactSheetLayout, validateContactSheetLayout, MEDIA2_MAX_PIXELS } from "./request.ts";
export { validateDerivative2Document, selectDerivative2Artifact } from "./receipt.ts";
export { Media2Client, Media2PreflightError, Media2RpcError, Media2TransportError, preflightMedia2Dispatch, launchMedia2Worker } from "./client.ts";
export type { Media2Transport, Media2JobInput, Media2JobStatus } from "./client.ts";
export type { Media2Handshake, Media2RpcRequest, Media2RpcResponse, Media2ErrorCode } from "./protocol.ts";
