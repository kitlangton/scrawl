import type { ChildProcessWithoutNullStreams } from "node:child_process";

/** Internal capability used only after negotiated recovery-role startup.
 * Closing pending local RPCs is separate from native process lifetime.
 */
export const releaseLocalRequests: unique symbol = Symbol("release recovery RPC ownership");

/** Local Node cleanup only. The caller separately owns an explicit close or a
 * negotiated recovery release; this neither grants authority nor proves exit.
 */
export function releaseProcessReferences(child: ChildProcessWithoutNullStreams): void {
  for (const stream of [child.stdin, child.stdout, child.stderr]) {
    stream.on("error", () => {});
    stream.destroy();
  }
  child.on("error", () => {});
  child.unref();
}
