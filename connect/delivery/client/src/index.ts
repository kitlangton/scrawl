export * from "./recipient/crypto.ts";
import { verifyEnvelope, type Context } from "./recipient/crypto.ts";
export { RecipientEnvelopeError } from "./recipient-envelope.ts";
export type { RecipientEnvelope, RecipientEnvelopeFailure } from "./recipient-envelope.ts";
import type { RecipientEnvelope } from "./recipient-envelope.ts";
import { validateRecipientEnvelopeWithVerifier, validateRecipientEnvelopeBytesWithVerifier } from "./recipient-envelope.ts";
export function validateRecipientEnvelope(input: unknown, expected: Context, trustedSenderSigningKey: Uint8Array): Promise<RecipientEnvelope> {
  return validateRecipientEnvelopeWithVerifier(input, expected, trustedSenderSigningKey, verifyEnvelope);
}
export function validateRecipientEnvelopeBytes(bytes: Uint8Array, expected: Context, trustedSenderSigningKey: Uint8Array): Promise<RecipientEnvelope> {
  return validateRecipientEnvelopeBytesWithVerifier(bytes, expected, trustedSenderSigningKey, verifyEnvelope);
}
export { openRecipientArchive, RecipientArchiveError } from "./recipient/archive.ts";
export type { RecipientArchiveInput, OpenedRecipientArchive } from "./recipient/archive.ts";
