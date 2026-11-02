// Platform bindings come from the official Workers declarations. SQL row types
// describe this Worker's migrations, not untrusted request/provider JSON.
export interface Env {
 DB: D1Database;
 BUCKET: R2Bucket;
 SITE_ORIGIN: string;
 LEGACY_SITE_ORIGIN?: string;
 API_ORIGIN: string;
 GOOGLE_CLIENT_ID: string;
 GOOGLE_CLIENT_SECRET: string;
 TOKEN_ENCRYPTION_KEY: string;
 BETTER_AUTH_SECRET: string;
 OAUTH_DISCOVERY_URL?: string;
 SHARING_ENABLED?: string;
}
export type Scope = 'read' | 'upload';
export type Beta = 'active' | 'waitlist';
export interface SessionRow {
 user_id: string;
 hash: string; account_id: string; csrf: string; expires_at: number;
 email: string; beta: Beta; interest: number;
}
export interface TokenRow {
 grant_id: string;
 hash: string; account_id: string; scope: Scope; expires_at: string; beta: Beta;
}
export interface ManifestFile { path: string; size: number; sha256: string }
export interface FileRow extends ManifestFile { capture_id: string; object_key: string | null }
export interface CaptureRow {
 reservation_seq: number; id: string; account_id: string; manifest: string; bytes: number;
 status: 'pending' | 'ready' | 'deleted'; expires_at: number; pending_until: number;
}
export interface AttemptSnapshot { key: string; terminal: number }
export type ShareState = 'pending' | 'ready' | 'paused' | 'failed' | 'expired' | 'revoked' | 'deleted';
export interface ShareRow {
 id: string; capture_id: string; account_id: string; hash: string; envelope: string;
 created_at: number; revoked_at: number | null; upload_state: 'uploading' | 'paused' | 'failed';
 status: CaptureRow['status']; expires_at: number; pending_until: number; bytes: number;
}
export interface SharedFile extends ManifestFile { url: string }
export interface ShareModel {
 url: string; status: ShareState; title: string; createdAt: string; expiresAt: string;
 uploadedBytes: number; totalBytes: number; files: SharedFile[]; thumbnailUrl: string | null;
 retryAfterSeconds: number | null; retryGuidance: string; evidenceNotice: string;
 summary?: string; transcript?: string;
}
