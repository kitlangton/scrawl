import { resolve, isAbsolute } from "node:path";
import { CaptureClient, ChildProcessCaptureTransport } from "../capture.ts";
import { MediaProcessTransport } from "../media.ts";
import { Media2Client } from "../media2/client.ts";
import { encodeMedia2Rpc, MEDIA2_MAX_LINE, type Media2RpcRequest, type Media2RpcResponse } from "../media2/protocol.ts";
import { releaseLocalRequests } from "./local.ts";
import { connectRecoveryPeer, type RecoveryPeer } from "./process.ts";
export type { RecoveryPeer, RecoveryPeerFacts, RecoveryPeerRelease } from "./process.ts";
export { RecoveryLaunchError, RECOVERY_RELEASE_MS, RECOVERY_STARTUP_MS } from "./process.ts";

function runtime(value: string): void {
  if (!isAbsolute(value) || resolve(value) !== value) throw new Error("Recovery runtime must be a normalized absolute path");
}
const refuseLegacyClose = async (): Promise<never> => { throw new Error("Recovery connection ownership belongs to peer.release(), not client.close()"); };

/** Requires an admitted artifact supporting the negotiated recovery lifetime.
 * After local admission is closed and local operations drained, release the
 * peer even when runtime clearance is unresolved. Never use this to detach an
 * already-owning live-capture client or infer NativeSaved/runtime clearance.
 * A never-injected peer may be released directly after a later launch fails.
 */
export function launchCaptureRecoveryPeer(executable: string, runtimeDirectory: string): Promise<RecoveryPeer<CaptureClient>> {
  runtime(runtimeDirectory);
  return connectRecoveryPeer(executable, ["--runtime-directory", runtimeDirectory, "--capture-channel", "capture", "--capture-protocol-version", "2"], "capture/2", child => {
    const transport = new ChildProcessCaptureTransport(child);
    const client = new CaptureClient({ request: (request, timeoutMs) => transport.request(request, timeoutMs), close: refuseLegacyClose });
    return { client, releaseLocal: () => transport[releaseLocalRequests]() };
  });
}
/** Same private runtime as capture/2; no new material/job identity or selection.
 * Caller still explicitly handshakes Media2Client before inspecting/deriving.
 */
export function launchMedia2RecoveryPeer(executable: string, runtimeDirectory: string): Promise<RecoveryPeer<Media2Client>> {
  runtime(runtimeDirectory);
  return connectRecoveryPeer(executable, ["--media-runtime-directory", runtimeDirectory, "--media-channel", "media", "--media-protocol-version", "2"], "media/2", child => {
    const transport = new MediaProcessTransport<Media2RpcRequest, Media2RpcResponse>(child, 3000, { version: 2, encode: encodeMedia2Rpc, maxLine: MEDIA2_MAX_LINE });
    const client = new Media2Client({ request: (request, timeoutMs) => transport.request(request, timeoutMs), close: refuseLegacyClose });
    return { client, releaseLocal: () => transport[releaseLocalRequests]() };
  });
}
