import type { RecordClient } from '@apparent/record/capabilities';

/** Trusted main-process binding; never constructed from renderer input. */
export interface RecordHostServices {
  record: RecordClient;
  /** Authoritative ownership/reconciliation clearance; missing or unknown blocks close. */
  canClose(): Promise<boolean>;
  /** Releases owned resources after clearance; must not silently stop captures. */
  close(): Promise<void>;
}
