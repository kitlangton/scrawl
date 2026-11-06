export type RecipientArchiveFailure =
  | "invalid_encryption_archive" | "encrypted_context_mismatch"
  | "invalid_encryption_parts" | "invalid_encryption_header"
  | "unsupported_encryption_header" | "encryption_decrypt_failed"
  | "encryption_plaintext_mismatch" | "invalid_manifest"
  | "unsafe_bundle_reference" | "quota_exceeded" | "cancelled";

/** Never include wire bytes, paths, key material or dependency diagnostics. */
export class RecipientArchiveError extends Error {
  readonly category: RecipientArchiveFailure;
  constructor(category: RecipientArchiveFailure) {
    super(category);
    this.name = "RecipientArchiveError";
    this.category = category;
  }
}
export function fail(category: RecipientArchiveFailure): never {
  throw new RecipientArchiveError(category);
}
