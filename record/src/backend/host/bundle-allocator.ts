import { createHash } from "node:crypto";
import { join } from "node:path";
import type { BundleAllocation } from "../engine/sdk-adapter.js";
import { ensurePrivateReservation, ensurePrivateRoot, hostError } from "./private-path.js";

/** Reserves an empty private Engine destination; it never creates a bundle artifact. */
export class PrivateBundleAllocator {
  readonly #root: string;

  constructor(privateBundleRoot: string) {
    this.#root = privateBundleRoot;
  }

  async allocate(requestId: string): Promise<BundleAllocation> {
    if (typeof requestId !== "string" || !requestId.length || Buffer.byteLength(requestId) > 4096) throw hostError();
    const root = await ensurePrivateRoot(this.#root);
    const digest = createHash("sha256").update(requestId, "utf8").digest("hex");
    const bundleId = `record-${digest}`;
    const bundlePath = join(root, `reservation-${digest}`);
    await ensurePrivateReservation(bundlePath, root, requestId);
    return { bundleId, bundlePath };
  }
}
