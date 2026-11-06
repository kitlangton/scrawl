/** Consumer-review proposal only: no SDK export, native advertisement or availability claim.
 * Record owns sampling, comparisons, thresholds, grouping, novelty and final selection.
 * Capture/2, media/1, media/2 and derivative/2 wire documents remain unchanged.
 */
import type { MaterialsPayload, SealedFile } from "../src/capture-documents/types.ts";
import type { Derivative2DocumentInput } from "../src/media2/types.ts";

export interface ProposedVisualObservationInput {
  /** Original prepared request/layout and detached receipt, not reconstructed expectations. */
  document: Derivative2DocumentInput;
  materials: import("../src/capture-documents/types.ts").MaterialsDocument;
  requestIndex: number; // Ordered, 1-based derivative frame selector; never a sheet selector.
  pngBytes: Uint8Array;
  maxFileBytes: number;
}

export interface ProposedVisualObservationSource {
  materials: import("../src/capture-documents/types.ts").MaterialsKey;
  leaseId: string;
  jobId: string;
  receiptId: string;
  receiptSha256: string;
  requestSha256: string;
  root: string;
  requestIndex: number;
  requestedBundleMs: number;
  effectiveVideoMs: number;
  actualVideoMs: number;
  clampedToStart: boolean;
  clampedToEnd: boolean;
  file: SealedFile; // Its hash/length cover the PNG pixels AND observation block.
  clock: MaterialsPayload["clock"];
  /** References/availability only. Fact bytes still need their existing SDK validation.
   * No events, settled marks, strokes or unavailable collections are inferred from pixels.
   */
  collection: MaterialsPayload["collection"];
  factRefs: MaterialsPayload["facts"];
}

export type ProposedFrameVisualObservation =
  | {
      state: "measured";
      schema: "apparent.frame-visual-observations/1";
      source: ProposedVisualObservationSource;
      pixels: {
        width: number;
        height: number;
        origin: "decoded-derivative-rgba8";
        alpha: "ignored";
      };
      algorithm: "apparent.visual-luma/1";
      thumbnail: {
        columns: 64;
        rows: 36;
        sampling: "cell-centres";
        /** Row-major native float32 values in [0,1], exposed as JS numbers. */
        cells: readonly number[];
      };
      regions: {
        columns: number; // min(actual decoded frame width, 128)
        rows: number; // min(actual decoded frame height, 128)
        sampling: "legacy-grid128-sampled-mean";
        cells: readonly number[];
      };
    }
  | {
      state: "unavailable";
      source: ProposedVisualObservationSource;
      reason: "observation-profile-absent" | "observation-profile-unsupported";
    };

/** Proposed public offline API for a later additive SDK patch, not implemented here.
 * It validates the original derivative/material/request/layout binding, selects the exact
 * frame, checks PNG byte length and full SHA-256 against its seal, then parses the profile.
 * Malformed data/CRC/dimensions/nonfinite or out-of-range cells fail closed, not as zeros.
 * Missing/unsupported profile is explicit unavailable for older native artifacts/replayed jobs.
 * This validates supplied data consistency; native issuance still comes from an authenticated
 * Media2Client completion and the native registry. A caller-authored receipt is not authority.
 * No filesystem reads, RPC, dispatch reservation, material release or cleanup occurs here.
 */
export declare function proposedReadFrameVisualObservations(
  input: ProposedVisualObservationInput,
): ProposedFrameVisualObservation;

/** Proposed native file profile: optional standard PNG ancillary/private/unsafe-to-copy
 * chunk `apOB`, before IEND, once per media/2 FRAME PNG (not contact sheets or media/1).
 * This is an explicit versioned file-output extension, not extra strict wire fields/methods.
 * Existing PNG readers ignore the ancillary block; existing derivative/2 seals cover it.
 * No path, credential, job ID or new material/candidate claim is embedded in PNG metadata.
 *
 * Block bytes, exact little endian:
 *   0..8   ASCII `APOBS001`
 *   8..12  decoded width u32
 *   12..16 decoded height u32
 *   16..18 region columns u16
 *   18..20 region rows u16
 *   20..   64*36 thumbnail f32, then columns*rows region f32, both row-major
 * Maximum 74,772 bytes, no compressed metadata and no unbounded decompression in the SDK.
 * PNG chunk lengths/CRC retain PNG's standard big-endian encoding.
 *
 * Measurements reuse keyframes.rs math on the ACTUAL decoded derivative pixel plane,
 * after requested native decode sizing, not a claim about unscaled source-video pixels.
 * Thumbnail samples cell centres with encoded-RGB (0.2126R+0.7152G+0.0722B)/255.
 * Regions preserve legacy min(width,128)*min(height,128), step=max(max(width,height)/512,1),
 * integer ((54R+183G+19B)>>8) cell mean /255. Alpha is ignored; no gamma linearization.
 * Existing frame pixel limits still apply; added metadata counts toward byte, deadline and
 * supervised resource budgets. Measurements never increase the admitted decoded pixel count.
 *
 * Proposed optional native artifact manifest advertisement:
 *   visualObservations: {schema:"apparent.frame-observation-profile/1",
 *     appliesTo:"media/2-frame-png",chunkType:"apOB",algorithm:"apparent.visual-luma/1"}
 * No extra process, worker, capture slot, protocol bypass or new output writer is introduced.
 * Capture candidates remain unavailable until a separate real collector exists.
 *
 * OPEN FOR RECORD REVIEW: accept this sealed PNG profile versus requiring a separately
 * negotiated observation document. No profile is frozen/activated by this proposal.
 */
