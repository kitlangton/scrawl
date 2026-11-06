import { runRecipientCommand, RecipientCommandError, type RecipientCommand, type RecipientCliResult } from "./recipient.ts";
import { openRecipientReadSession, RecipientSessionError, type RecipientReadSession } from "../state/recipient-session.ts";
import { publishRecipientCapture, RecipientPublicationError, type RecipientPublicationResources } from "../recipient/publication.ts";
import type { NamespaceResources } from "../state/namespace.ts";

export type StateRecipientOptions = {
  directory: string;
  /** Already approved by the host; URL validation does not establish approval. */
  approvedServer: string;
  resources: NamespaceResources & RecipientPublicationResources;
  inspectLocal(source: string): Promise<string>;
  signal?: AbortSignal;
  nowSeconds?: () => number;
  /** Explicit legacy policy, never inferred from a missing or damaged key. */
  allowLegacyPlaintext?: boolean;
};
function commandError(error: unknown): RecipientCommandError {
  if (error instanceof RecipientSessionError) return new RecipientCommandError(error.category);
  if (error instanceof RecipientPublicationError) {
    if (error.category === "cancelled" || error.category === "destination_exists" || error.category === "publication_refused"
      || error.category === "unsafe_directory") return new RecipientCommandError(error.category);
  }
  return new RecipientCommandError("operation_failed");
}

/** Actual state-backed command composition; does not install a provider or route
 * an installed CLI. Only the legacy empty lock file is created in shared state. */
export async function runRecipientCommandFromState(command: RecipientCommand, options: StateRecipientOptions): Promise<RecipientCliResult> {
  const { directory, approvedServer, resources, inspectLocal, signal, nowSeconds, allowLegacyPlaintext } = options;
  command = { ...command };
  let session: RecipientReadSession | undefined;
  try {
    for (let attempt = 0; attempt < 2; attempt++) {
      const result = await runRecipientCommand(command, {
        cancelled: signal, allowLegacyPlaintext, inspectLocal,
        async loadCredential() {
          try { session ??= await openRecipientReadSession(directory, approvedServer, resources, { signal, nowSeconds }); return session.credential; }
          catch (error) { throw commandError(error); }
        },
        async resolveIdentity() {
          try { if (!session) throw new RecipientCommandError("operation_failed"); return await session.resolveIdentity(); }
          catch (error) { throw commandError(error); }
        },
        async publish(capture, destination) {
          try {
            if (!session) throw new RecipientCommandError("operation_failed");
            await session.assertCurrent();
            return await publishRecipientCapture(capture, destination, resources, { signal });
          } catch (error) { throw commandError(error); }
          finally { for (const file of capture.files) file.bytes.fill(0); }
        },
      });
      if (result.ok || result.error.category !== "unauthorized" || attempt === 1 || !session) return result;
      try {
        const next = await session.reloadAfterUnauthorized();
        session.dispose(); session = next;
      } catch (error) {
        const { category } = commandError(error);
        return { ok: false, error: { category, message: category } };
      }
    }
    return { ok: false, error: { category: "operation_failed", message: "operation_failed" } };
  } finally { session?.dispose(); }
}
