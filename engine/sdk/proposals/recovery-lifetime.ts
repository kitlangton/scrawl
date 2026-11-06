/** Consumer-review proposal only. No export, native role or safe-release claim.
 * Existing capture/2 and media/2 wire methods and default close() stay unchanged.
 * The existing native processes retain their admitted finite work; no new daemon,
 * worker framework, job identity, cancellation or capture admission is introduced.
 */
import type { CaptureClient } from "../src/capture.ts";
import type { Media2Client } from "../src/media2/index.ts";

export interface ProposedRecoveryPeerFacts {
  schema: "apparent.engine-recovery-peer/1";
  /** Fresh private launch nonce, not a capture/material/job/publication identity. */
  peerId: string;
  channel: "capture/2" | "media/2";
  implementation: "native";
  role: "recovery";
  /** Native enforcement, not merely omission of a TS start method. */
  startsFreshCapture: false;
  onInputEof: "retain-admitted-finite-work";
}

export interface ProposedRecoveryPeerRelease {
  scope: "client-connection";
  peerId: string;
  /** Positive local guarantee even when the native EOF observation is lost:
   * SDK-owned process/stdio handles, pending-RPC timers and lifecycle timers no
   * longer keep the caller's Node event loop alive. Not native process exit.
   */
  connection: "released";
  /** Unknown is not permission to kill/cancel native work or allocate a new tuple. */
  inputClosure: "confirmed" | "unconfirmed" | "peer-exit-observed";
  runtimeClearance: "not-established";
  reconcile: "original-capture-materials-job-publication-bindings";
}

export interface ProposedRecoveryPeer<Client> {
  /** Inject this normal public client into Record's existing coordinator.
   * The separate peer handle owns its connection release. Do not call legacy
   * client.close() to release an unresolved recovery operation.
   */
  readonly client: Client;
  readonly lifetime: ProposedRecoveryPeerFacts;
  /** Call after Record shutdown has closed local admission and drained its local
   * operations. This releases connection ownership only; it neither changes nor
   * overrides Record runtimeClearance, NativeSaved or any job outcome.
   * A peer never injected into a coordinator may be released directly. A failed
   * shutdown or unknown local drain does not establish this caller precondition.
   *
   * Bound: a fixed documented release deadline (proposed <=5s), never dependent
   * on media/publication completion. No timeout-as-cancel, TERM/KILL escalation,
   * material release or private child-handle operation is exposed to the caller.
   * Native finite work retains its existing owner, leases, deadlines and durable
   * original identity, then the existing native process exits when work ends.
   * A response lost during release remains unconfirmed, not runtime-clear.
   * Idempotent and concurrent-call safe: callers receive one stable outcome.
   * New client operations reject locally once release begins, before writing.
   * Pending response loss remains original-tuple uncertainty, never cancellation.
   * The operation always performs local resource release within the bound; an
   * unconfirmed inputClosure cannot conceal retained caller event-loop handles.
   */
  release(): Promise<ProposedRecoveryPeerRelease>;
}

/** Proposed launch APIs require an admitted artifact advertising the new lifetime
 * seam. Native startup must confirm the exact peer nonce/role before exposing a
 * usable client. An old artifact is unsupported, never silently safe-closeable.
 * No new job/capture/selection is created by these factories.
 * Startup has a fixed <=5s negotiation bound. Failed negotiation cleans up only
 * the newly launched, never-exposed process/connection, to which no operation
 * was dispatched. It cannot call legacy close on an earlier successful peer.
 * If a second launch or coordinator construction fails, the caller releases any
 * earlier returned peer handle directly, without an artificial shutdown report.
 */
export declare function proposedLaunchCaptureRecoveryPeer(
  executable: string,
  runtimeDirectory: string,
): Promise<ProposedRecoveryPeer<CaptureClient>>;
export declare function proposedLaunchMedia2RecoveryPeer(
  executable: string,
  runtimeDirectory: string,
): Promise<ProposedRecoveryPeer<Media2Client>>;

/** Required native boundary, not implemented by this proposal:
 * - Explicit opt-in launch role/lifecycle version, separately negotiated from the
 *   frozen capture/2 and media/2 request/result shapes. Exact argv/event spelling
 *   remains open until transport implementation review. Legacy launch unchanged.
 * - Capture recovery role rejects Start before binding/admission; EOF never stops
 *   another peer's live capture. Recovery may inspect/control original captures
 *   and resume only already-authorized finite processing/publication operations.
 *   Its normal CaptureClient handshake also reports fresh-start readiness
 *   unavailable and displayCapture=false; role metadata alone is insufficient.
 * - Media EOF drains admitted work under its original deadline and supervision,
 *   rather than manufacturing owner_lost because the observer disconnected.
 * - Publication retains original auth/material/plan bindings and held resources
 *   through commit/recovery. Releasing the client is never commit/cancel/cleanup.
 * - Ready/release facts bind the private launch nonce. Releasing closes admission
 *   to this connection, not the runtime. Already received requests may still be
 *   processed; they retain original authorization and identities. Unknown work
 *   stays reconcilable. Input closure is not proof that native job admission,
 *   work completion or global runtime clearance has finished.
 *
 * Agent must use only this public peer handle, with Record local shutdown/drain;
 * no unref/process.exit/private-child/force-close workaround at the consumer.
 * Record's existing default-host SDK ownership rule stays intact: unresolved
 * owning clients cannot use legacy close. A recovery-peer release is a distinct
 * native ownership handoff, not permission to report runtimeClearance='clear'.
 * Calling the returned client.close() refuses locally without signaling native
 * work, including after release; only peer.release() owns this connection.
 *
 * OPEN: prove bounded public release and process exit without consumer hacks,
 * active capture unaffected, finite jobs/publication retained, lost ACK handling,
 * reaping, and cold original-tuple inspection against real native artifacts on
 * each supported target. Type declarations alone establish none of those facts.
 */
