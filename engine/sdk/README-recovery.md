# Recovery connection ownership

SDK 0.4.3 exports `@apparent/engine-sdk/recovery`. The export requires a matching
admitted native recovery artifact; it does not upgrade earlier native packages.
Native qualification and per-platform delivery remain separate adoption gates.
Existing SDK versions and their immutable tarballs remain unchanged.

`launchCaptureRecoveryPeer` and `launchMedia2RecoveryPeer` return a normal client,
immutable native lifetime facts and a separately owned `release()` handle. They
append `--recovery-peer-v1 <private nonce>` to the existing six launch arguments.
No RPC is sent before an exact nonce/channel/role startup fact is received from
the new process. Capture's normal handshake must also report capture unavailable;
the native role rejects fresh start before durable binding or admission.

The owner injects `.client` into the existing Record coordinator. Before release,
it closes local admission, stops other borrowers and positively drains local
operations through Record shutdown/dispose. An unknown or failed local drain is
not permission to release. A peer never injected into a coordinator can be
released directly, including when construction of the other peer fails.

Release synchronously refuses new RPCs and rejects pending responses as unknown.
It ends input, waits at most 2 seconds for an input-closed fact or observed exit,
then releases the SDK's process/stdio references and pending/lifecycle timers.
Repeated/concurrent calls share one stable result. Every resolved result's
`connection: released` guarantees those SDK resources no longer keep Node alive.
`inputClosure: unconfirmed` describes a missing native observation, not retained
local handles. Neither a confirmed EOF nor observed process exit establishes
runtime clearance, successful publication, cancellation or release of materials.
Already received requests can still finish under their original durable tuple.

The native recovery role must retain existing finite processing/publication
authority, leases, deadlines and supervision after EOF or output disconnection,
then exit when its work ends. It must not own an unbounded fresh recording.
`unref` inside the SDK merely relinquishes a Node process reference after native
role negotiation; it does not establish these native safety properties. Callers
receive no child handle and need no exit/unref/signal workaround. The returned
client's `close()` rejects locally, both before and after release, so it cannot
create legacy signal timers against a handed-off worker.

Startup negotiation is bounded to 4 seconds. On failure, no client was exposed
and no RPC dispatched. SDK cleanup signals only that fresh launch (TERM, then
KILL after 500 ms), waits at most 1 second, and releases local handles before
rejecting `RecoveryLaunchError`. Its `nativeExitObserved` distinguishes actual
exit evidence from an unobserved exit; neither case affects an earlier peer.
No unsupported artifact falls back to legacy close or becomes a usable client.

The source subprocess tests prove local timer/handle release, normal one-shot
exit, lost-ACK handling, failed second launch and nonce/startup rejection using
an explicitly synthetic process fixture. They do not prove native continuation,
reconnect authority, capture isolation, reaping or either platform's artifact.
