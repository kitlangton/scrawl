import { lstat, realpath, readdir } from "node:fs/promises";
import path from "node:path";
import { normalizeRecipientServer } from "../recipient/transport.ts";
import { readCancellation, readCredential, pendingIntent, readStateEvidence, stateFileStamp } from "../state/credential-files.ts";
import { CredentialSnapshotError, checkSignal, samePathFd, withStateQueue, type NamespaceResources } from "../state/queue-transaction.ts";
import { replacePrivateStateFile, type StateReplacementResources } from "../state/replacement.ts";
import { decodeJob, encodeJob, type Job } from "./job.ts";
import { planAdmission, ADMISSION_MAX_JOBS, type AdmissionIntent, type AdmissionProposal } from "./admission.ts";
import { readQueueWithinLease } from "./store.ts";
import { retireQueuedJob } from "./prune.ts";
import type { UploadWorkerSettings } from "./worker.ts";

export type AdmissionStoreResources = NamespaceResources & StateReplacementResources;
export type AdmissionOutcome = Readonly<{ state: "admitted" | "reusable"; id: string; cancelledJobIds: readonly string[]; retiredJobIds: readonly string[] }>
  | Readonly<{ state: "full" | "refused" | "uncertain"; admittedJobId?: string; cancelledJobIds: readonly string[]; retiredJobIds: readonly string[] }>;
export type AdmissionStoreInput = Readonly<{ directory: string; approvedServer: string; bundle: string; intent: AdmissionIntent; candidateId: string;
  readSettings(signal: AbortSignal): Promise<UploadWorkerSettings>; signal?: AbortSignal; queueTimeoutMs?: number }>;
const frozen = <T>(v: T): T => { if (v && typeof v === "object" && !Object.isFrozen(v)) { Object.freeze(v); for (const x of Object.values(v as object)) frozen(x); } return v; };
const idOk = (v: unknown): v is string => typeof v === "string" && /^[0-9a-f]{32}$/.test(v);
const epoch = (v: string | null) => JSON.stringify(v);
type BundlePin = { directory: string; dirStamp: string; manifest: string; manifestStamp: string };
async function bundlePin(value: string): Promise<BundlePin> {
  if (typeof value !== "string" || !path.isAbsolute(value) || value.includes("\0")) throw Error();
  const directory = await realpath(value), ds = await lstat(directory, { bigint: true });
  if (path.extname(directory) !== ".apparent" || !ds.isDirectory() || ds.isSymbolicLink()) throw Error();
  const manifest = path.join(directory, "apparent.json"), ms = await lstat(manifest, { bigint: true });
  if (!ms.isFile() || ms.isSymbolicLink() || ms.nlink < 1n) throw Error();
  return { directory, dirStamp: stamp(ds), manifest, manifestStamp: stamp(ms) };
}
const stamp = (s: { dev: bigint; ino: bigint; mode: bigint; uid: bigint; size: bigint; mtimeNs: bigint; ctimeNs: bigint; nlink: bigint }) => `${s.dev}:${s.ino}:${s.mode}:${s.uid}:${s.size}:${s.mtimeNs}:${s.ctimeNs}:${s.nlink}`;

/** Internal composition only. Rust remains the installed sole queue writer. */
export async function admitQueuedUpload(input: AdmissionStoreInput, resources: AdmissionStoreResources): Promise<AdmissionOutcome> {
  const directory = input?.directory, approvedServer = input?.approvedServer, source = input?.bundle, intent = input?.intent,
    candidateId = input?.candidateId, readSettings = input?.readSettings, callerSignal = input?.signal, queueTimeoutMs = input?.queueTimeoutMs;
  const cancelled: string[] = [], retired: string[] = [];
  let admittedJobId: string | undefined;
  const result = (state: AdmissionOutcome["state"], id?: string): AdmissionOutcome => frozen({ state, ...(id ? { id } : {}), ...(state === "uncertain" && admittedJobId ? { admittedJobId } : {}), cancelledJobIds: [...cancelled], retiredJobIds: [...retired] } as AdmissionOutcome);
  let dispatched = false, effects = false;
  try {
    const server = normalizeRecipientServer(approvedServer);
    if (!idOk(candidateId) || !["automatic-private", "explicit-private", "public-copy"].includes(intent) || typeof readSettings !== "function") return result("refused");
    return await withStateQueue(directory, { scope: "upload", approvedServer: server }, resources, { signal: callerSignal, lockTimeoutMs: queueTimeoutMs }, async (root, lease) => {
      const signal = AbortSignal.any([lease.signal, ...(callerSignal ? [callerSignal] : [])]);
      const check = async () => { checkSignal(signal, lease.signal); await lease.assertCurrent(); if (!await samePathFd(root)) throw Error(); };
      const settings = async (): Promise<UploadWorkerSettings> => {
        await check(); let raw: UploadWorkerSettings;
        try { raw = await readSettings(signal); } catch { throw Error(); }
        await check();
        let normalized = ""; try { normalized = normalizeRecipientServer(raw?.server); } catch { throw Error(); }
        if (typeof raw?.enabled !== "boolean" || normalized !== server || typeof raw.encryptionEnabled !== "boolean" || typeof raw.sharingEnabled !== "boolean"
          || !Number.isSafeInteger(raw.nowSeconds) || raw.nowSeconds < 0 || raw.nowSeconds > Number.MAX_SAFE_INTEGER - 3600
          || !Number.isSafeInteger(raw.retentionSeconds) || raw.retentionSeconds < 0 || !raw.enabled || intent === "public-copy" && !raw.sharingEnabled) throw Error();
        return Object.freeze({ enabled: raw.enabled, server: normalized, encryptionEnabled: raw.encryptionEnabled, sharingEnabled: raw.sharingEnabled, nowSeconds: raw.nowSeconds, retentionSeconds: raw.retentionSeconds });
      };
      const settingsPin = await settings();
      await check();
      const pin = await bundlePin(source);
      await check();
      if (await pendingIntent(root.path)) throw Error();
      const credential = await readCredential(root, { scope: "upload", approvedServer: server }, resources, signal);
      const cancellation = epoch(await readCancellation(root, resources, signal));
      if (await pendingIntent(root.path)) throw Error();
      const snapshot = await readQueueWithinLease(root, lease, resources, signal);
      let expectedJobs: readonly Job[] = snapshot;
      const candidateAbsent = async () => {
        const names = await readdir(root.path);
        return !snapshot.some(j => j.id === candidateId) && !names.includes(`${candidateId}.job`) && !names.includes(`${candidateId}.sealed`);
      };
      if (!await candidateAbsent()) return result("refused");
      const planning = (jobs: readonly Job[], p = settingsPin) => planAdmission({ intent, bundle: pin.directory, server, generation: credential.generation,
        now: p.nowSeconds, retentionSeconds: p.retentionSeconds, enabled: p.enabled, sharingEnabled: p.sharingEnabled,
        encryptionEnabled: p.encryptionEnabled, candidateId, jobs });
      const plan = planning(snapshot);
      const recheck = async (expected: readonly Job[] = expectedJobs) => {
        await check(); const p = await settings();
        if (p.enabled !== settingsPin.enabled || p.server !== settingsPin.server || p.encryptionEnabled !== settingsPin.encryptionEnabled
          || p.sharingEnabled !== settingsPin.sharingEnabled || p.retentionSeconds !== settingsPin.retentionSeconds || await pendingIntent(root.path)) throw Error();
        const c = await readCredential(root, { scope: "upload", approvedServer: server }, resources, signal);
        if (c.generation !== credential.generation || c.grantId !== credential.grantId || c.encryption_identity !== credential.encryption_identity
          || epoch(await readCancellation(root, resources, signal)) !== cancellation) throw Error();
        await check(); const b = await bundlePin(source); await check();
        if (b.directory !== pin.directory || b.dirStamp !== pin.dirStamp || b.manifestStamp !== pin.manifestStamp) throw Error();
        const latest = await readQueueWithinLease(root, lease, resources, signal);
        if (JSON.stringify(latest) !== JSON.stringify(expected)) throw Error();
        if (!await candidateAbsent()) throw Error();
        return { p, latest };
      };
      const proposals: readonly AdmissionProposal[] = plan.proposals;
      for (const proposal of proposals) {
        if (proposal.kind === "cancel") {
          const saved = await readStateEvidence(root, `${proposal.job.id}.job`, 2_000_000, resources, signal, decodeJob);
          if (!saved || saved.value.status === "ready") throw Error();
          const bytes = encodeJob(proposal.job); const named = await lstat(saved.file, { bigint: true });
          if (stateFileStamp(named) !== saved.expectedStamp) throw Error();
          const guarded: StateReplacementResources = { checkAcl: h => resources.checkAcl(h), async replaceFile(parent, ...args) {
            try { await recheck(expectedJobs); } catch { await parent.close().catch(() => {}); return { state: "refused" }; }
            dispatched = true; return resources.replaceFile(parent, ...args);
          } };
          try {
            const replaced = await replacePrivateStateFile(root.path, `${proposal.job.id}.job`, bytes, { device: String(named.dev), inode: String(named.ino) },
              { signal, assertHeld: () => { checkSignal(signal, lease.signal); lease.assertHeld(); } }, guarded, { signal });
            if (replaced.state !== "committed") return result(effects || dispatched || replaced.state === "publication_uncertain" ? "uncertain" : "refused");
            effects = true; cancelled.push(proposal.job.id); dispatched = false;
            expectedJobs = expectedJobs.map(job => job.id === proposal.job.id ? proposal.job as Job : job);
          } finally { bytes.fill(0); }
        } else {
          const fresh = await readStateEvidence(root, `${proposal.id}.job`, 2_000_000, resources, signal, decodeJob);
          if (!fresh) throw Error();
          const job = fresh.value;
          const assertFresh = async () => {
            const { latest, p } = await recheck(expectedJobs);
            if (planning(latest, p).proposals.find(x => x.kind === "prune" && x.id === proposal.id) === undefined) throw Error();
          };
          dispatched = true;
          const retiredResult = await retireQueuedJob(root, lease, resources, { job, expectedStamp: fresh.expectedStamp, assertFresh });
          if (retiredResult.state !== "retired") return result(effects || retiredResult.state === "uncertain" ? "uncertain" : "refused");
          effects = true; retired.push(proposal.id); dispatched = false;
          expectedJobs = expectedJobs.filter(job => job.id !== proposal.id);
        }
      }
      const { latest } = await recheck(expectedJobs);
      if (latest.length > ADMISSION_MAX_JOBS || latest.some(j => j.id === candidateId)) return result("uncertain");
      const finalPlan = planning(latest);
      if (finalPlan.kind === "reusable") return result("reusable", finalPlan.id);
      if (finalPlan.kind === "full") return result("full");
      if (finalPlan.kind !== "admit") return result("refused");
      const bytes = encodeJob(finalPlan.job);
      const guarded: StateReplacementResources = { checkAcl: h => resources.checkAcl(h), async replaceFile(parent, ...args) {
        try {
          const { latest: fresh } = await recheck(expectedJobs);
          if (fresh.length >= ADMISSION_MAX_JOBS || fresh.some(j => j.id === candidateId)) { await parent.close(); return { state: "refused" }; }
          const currentPlan = planning(fresh); if (currentPlan.kind !== "admit" || JSON.stringify(currentPlan.job) !== JSON.stringify(finalPlan.job)) { await parent.close(); return { state: "refused" }; }
        } catch { await parent.close().catch(() => {}); return { state: "refused" }; }
        dispatched = true; return resources.replaceFile(parent, ...args);
      } };
      try {
        const outcome = await replacePrivateStateFile(root.path, `${candidateId}.job`, bytes, null,
          { signal, assertHeld: () => { checkSignal(signal, lease.signal); lease.assertHeld(); } }, guarded, { signal });
        if (outcome.state === "committed") { effects = true; dispatched = false; admittedJobId = candidateId; return result("admitted", candidateId); }
        return result(effects || dispatched || outcome.state === "publication_uncertain" ? "uncertain" : "refused");
      } finally { bytes.fill(0); }
    });
  } catch (e) {
    if (dispatched || effects) return result("uncertain");
    if (e instanceof CredentialSnapshotError && e.category === "cancelled") return result("refused");
    return result("refused");
  }
}
