// Review boundary: Record owns capture-to-handoff policy and Engine owns native
// execution; these detached-document validators create no transport or client API.
// Materials are not completed bundles. Hashes bind exact detached UTF-8 bytes,
// and new-channel publication plans accept only ABSENT destinations. Full schema
// freeze, native enforcement, filesystem durability and crash recovery remain open.
// Record's coordinator persists the publication identity/plan binding before
// dispatch and never allocates or creates the final destination.
export * from "./types.ts";
export { MAX_DOCUMENT_BYTES } from "./common.ts";
export { validateMaterialsDocument, validateStrokeFacts } from "./materials.ts";
export { validateDerivativeEnvelope } from "./derivative.ts";
export { validateInferenceEnvelope } from "./inference.ts";
export { validatePublicationPlan } from "./publication.ts";
