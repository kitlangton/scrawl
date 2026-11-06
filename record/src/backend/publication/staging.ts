import { createHash } from "node:crypto";
import { constants, type BigIntStats } from "node:fs";
import { lstat, mkdir, open, realpath, readdir } from "node:fs/promises";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { isDeepStrictEqual } from "node:util";
import { preflightCapturePublication, type CapturePublishInput } from "@apparent/engine-sdk/capture";
import type { BundleAssembly } from "../processing/bundle/index.js";
import type { MaterialsDocument, PublicationPlan, PublishEnvelope } from "@apparent/engine-sdk/capture";
import { validateMaterialsDocument } from "@apparent/engine-sdk/capture";
import { selectDerivative2Artifact, validateDerivative2Document, type Derivative2DocumentInput, type Derivative2Selector } from "@apparent/engine-sdk/media2";
type DerivativeDocumentInput = NonNullable<CapturePublishInput["derivatives"]>[number];
type PublicationEntry = PublicationPlan["entries"][number];
type SealedFile = { relativePath: string; identity: { device: string; inode: string; size: number; modifiedNs: string }; sha256: string };
export type Derivative2ArtifactBinding = Readonly<Record<string, { receiptId: string; selector: Derivative2Selector }>>;

export interface PreparePublicationInput {
  ownerToken: string; publicationId: string; stagingRoot: string; destinationPath: string;
  materialsManifest: unknown; materialsPayloadBytes: Uint8Array; assembly: BundleAssembly;
  artifactBytes: Readonly<Record<string, Uint8Array>>;
  derivatives?: readonly DerivativeDocumentInput[];
  derivatives2?: readonly Derivative2DocumentInput[];
  derivative2Artifacts?: Derivative2ArtifactBinding;
}
/** inventory retains assembly/source metadata; actual staging identities are in plan.entries. */
export interface PreparedRecordPublication { input: CapturePublishInput; plan: PublicationPlan; inventory: BundleAssembly["inventory"] }
const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const id = /^[0-9a-f]{32}$/u;
const digest = /^[0-9a-f]{64}$/u;
const pathRef = (value: string) => !!value && !isAbsolute(value) && !value.includes("\\") && !value.includes("\0") && value.split("/").every(part => !!part && part !== "." && part !== "..");
const privateMode = 0o700;
const pending = new Map<string, { fingerprint: string; promise: Promise<PreparedRecordPublication> }>();

/** Persist a private SDK publication plan for review/consumption. This never dispatches publication. */
export async function prepareRecordPublication(input: PreparePublicationInput): Promise<PreparedRecordPublication> {
  input = structuredClone(input);
  const fingerprint = hash(new TextEncoder().encode(JSON.stringify({ captureId: input.assembly.materials.manifest.key.captureId,
    materials: input.assembly.materials.manifest.key, leaseId: input.assembly.materials.payload.leaseId, destinationPath: input.destinationPath, ownerToken: hash(new TextEncoder().encode(input.ownerToken)),
    stagingRoot: input.stagingRoot, inventory: input.assembly.inventory, bundlePath:input.assembly.bundlePath, assemblyMaterials:input.assembly.materials,
    materialsManifest:input.materialsManifest, materialsPayloadSha256:hash(input.materialsPayloadBytes),
    apparentJsonSha256:hash(input.assembly.apparentJson), summarySha256:hash(input.assembly.summaryMd),
    artifacts: Object.entries(input.artifactBytes).map(([path, bytes]) => [path, hash(bytes)]).sort(),
    derivatives: input.derivatives?.map(d => ({envelope:d.envelope,bytesSha256:hash(d.bytes),requestedBundleMs:d.requestedBundleMs})) ?? [],
    derivatives2: derivative2Fingerprint(input.derivatives2, input.derivative2Artifacts) })));
  const active = pending.get(input.publicationId);
  if (active) return active.fingerprint === fingerprint ? active.promise.then(result=>structuredClone(result)) : Promise.reject(new Error("Record publication identity conflicts with its in-flight staging binding"));
  const operation = prepare(input).finally(() => pending.delete(input.publicationId));
  pending.set(input.publicationId, { fingerprint, promise: operation });
  return operation.then(result=>structuredClone(result));
}

async function prepare(input: PreparePublicationInput): Promise<PreparedRecordPublication> {
  const materials = validateMaterialsDocument(input.materialsManifest, input.materialsPayloadBytes);
  if (!id.test(input.publicationId) || !digest.test(input.ownerToken) || !isDeepStrictEqual(input.assembly.materials,materials) ||
      !isDeepStrictEqual(input.assembly.inventory.materials,materials.manifest.key)) throw new Error("Record staging identity differs from validated materials");
  const derivative2ByReceipt = new Map<string, { input: Derivative2DocumentInput; envelope: ReturnType<typeof validateDerivative2Document> }>();
  if ((input.derivatives2 === undefined) !== (input.derivative2Artifacts === undefined)) throw new Error("Record derivative/2 documents and artifact bindings must be supplied together");
  for (const document of input.derivatives2 ?? []) {
    const envelope = validateDerivative2Document(document, materials);
    if (derivative2ByReceipt.has(envelope.receiptId)) throw new Error("Record derivative/2 receipt IDs must be unique");
    derivative2ByReceipt.set(envelope.receiptId, { input: document, envelope });
  }
  if (input.derivative2Artifacts) {
    for (const path of Object.keys(input.derivative2Artifacts)) if (!input.assembly.inventory.entries.some(entry => entry.path === path)) throw new Error(`Record derivative/2 mapping references unknown inventory path ${path}`);
  }
  const resolvedDerivative2 = new Map<string, { receipt: ReturnType<typeof validateDerivative2Document>; selector: Derivative2Selector }>();
  const usedDerivative2Bindings = new Set<string>();
  for (const item of input.assembly.inventory.entries) {
    const binding = input.derivative2Artifacts?.[item.path];
    if (input.derivatives2 && (item.role === "frame" || item.role === "contact-sheet") && !binding) throw new Error(`Record derivative/2 mapping is required for ${item.path}`);
    if (!binding) continue;
    if (item.role !== "frame" && item.role !== "contact-sheet") throw new Error(`Record derivative/2 mapping targets unsupported inventory role ${item.role}`);
    const receipt = derivative2ByReceipt.get(binding.receiptId);
    if (!receipt) throw new Error(`Record derivative/2 mapping references unknown receipt ${binding.receiptId}`);
    const selected = selectDerivative2Artifact(receipt.envelope, binding.selector);
    if (item.role !== (binding.selector.kind === "frame" ? "frame" : "contact-sheet") ||
        selected.file.sha256 !== item.sha256 || selected.byteLength !== item.byteLength || !isDeepStrictEqual(selected.file.identity, item.identity))
      throw new Error(`Record derivative/2 selection does not match assembled role, hash, length, and original file identity for ${item.path}`);
    const artifact = input.artifactBytes[item.path];
    if (!artifact || hash(artifact) !== item.sha256 || artifact.byteLength !== item.byteLength) throw new Error(`Record derivative/2 bytes differ from the original assembled artifact ${item.path}`);
    const selectorKey = `${binding.receiptId}:${binding.selector.kind === "frame" ? `frame:${binding.selector.requestIndex}` : "contact-sheet"}`;
    if (usedDerivative2Bindings.has(selectorKey)) throw new Error("Record derivative/2 selector cannot be bound to multiple final paths");
    usedDerivative2Bindings.add(selectorKey);
    resolvedDerivative2.set(item.path, canonicalDerivative2Keys({ receipt: receipt.envelope, selector: binding.selector }));
  }
  if (input.derivative2Artifacts) {
    const expected = new Set([...derivative2ByReceipt].flatMap(([receiptId, { envelope }]) => [
      ...envelope.payload.frames.map(frame => `${receiptId}:frame:${frame.requestIndex}`),
      ...(envelope.payload.contactSheet ? [`${receiptId}:contact-sheet`] : []),
    ]));
    for (const key of expected) if (!usedDerivative2Bindings.has(key)) throw new Error("Record derivative/2 documents contain an unused output");
  }
  const root = input.stagingRoot;
  const destination = input.destinationPath;
  if (!isAbsolute(root) || resolve(root) !== root || !isAbsolute(destination) || resolve(destination) !== destination || root === destination) throw new Error("Record staging and destination paths must be absolute and normalized");
  if (root.startsWith(`${destination}/`) || destination.startsWith(`${root}/`)) throw new Error("Record staging and destination paths overlap");
  if (input.assembly.bundlePath !== destination) throw new Error("Record staging destination differs from assembled bundle paths");
  await assertDestinationParents(destination);
  try { await lstat(destination); throw new Error("Record publication destination already exists"); }
  catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
  await ensurePrivateDirectory(root);
  const work = join(root, input.publicationId);
  try { await mkdir(work, { mode: privateMode }); await syncDirectory(root); }
  catch (error) { if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw new Error("Record staging namespace is unavailable"); }
  const stat = await lstat(work);
  if (!stat.isDirectory() || stat.isSymbolicLink() || stat.uid !== process.getuid?.() || (stat.mode & 0o077) !== 0) throw new Error("Record staging namespace is not private");
  await syncDirectory(root);
  const marker = join(work, ".record-publication");
  try { await lstat(marker); }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    if ((await readdir(work)).length) throw new Error("Record staging namespace has unrelated unbound state; preserving it for reconciliation");
  }
  const markerBinding = { publicationId: input.publicationId, materials: materials.manifest.key, leaseId: materials.payload.leaseId, destination,
    materialsManifestSha256:hash(new TextEncoder().encode(JSON.stringify(materials.manifest))),
    ownerTokenSha256: hash(new TextEncoder().encode(input.ownerToken)), inventorySha256: hash(new TextEncoder().encode(JSON.stringify(input.assembly.inventory))),
    artifactsSha256: hash(new TextEncoder().encode(JSON.stringify(Object.entries(input.artifactBytes).map(([path, bytes]) => [path, hash(bytes)]).sort()))),
    apparentJsonSha256:hash(input.assembly.apparentJson), summarySha256:hash(input.assembly.summaryMd),
    derivativesSha256:hash(new TextEncoder().encode(JSON.stringify(input.derivatives??[]))),
    ...(input.derivatives2 === undefined ? {} : { derivatives2Sha256:hash(new TextEncoder().encode(JSON.stringify(derivative2Fingerprint(input.derivatives2, input.derivative2Artifacts)))) }) };
  const markerBytes = new TextEncoder().encode(JSON.stringify({ schema: "apparent.record-staging/1", ...markerBinding }));
  await writeExclusiveOrMatch(marker, markerBytes);
  const entries: PublicationEntry[] = [];
  const assembledBytes = new Map<string, Uint8Array>([["apparent.json", input.assembly.apparentJson], ["summary.md", input.assembly.summaryMd]]);
  const ownedFiles = new Set([".record-publication", "publication-plan.json", ...input.assembly.inventory.entries.filter(entry => entry.provenance !== "material-reference").map(entry => entry.path)]);
  await assertKnownStagingContents(work, ownedFiles);
  const materialByPath = new Map(materials.payload.files.map(file => [file.relativePath, file]));
  for (const item of input.assembly.inventory.entries) {
    if (!pathRef(item.path)) throw new Error("Record staging inventory contains an invalid relative path");
    const role = item.role as PublicationEntry["role"];
    if (item.provenance === "material-reference") {
      const source = materialByPath.get(item.path);
      if (!source || source.sha256 !== item.sha256 || source.identity.size !== item.byteLength) throw new Error("Record staging material reference differs from sealed materials");
      entries.push({ relativePath: item.path, sha256: item.sha256, byteLength: item.byteLength, role, input: { kind: "material", relativePath: item.path } });
      continue;
    }
    // Assembly returns hashes/lengths, not caller-attested image bytes. Re-supply those exact bytes.
    const derivative2 = resolvedDerivative2.get(item.path);
    if (derivative2) {
      entries.push({ relativePath: item.path, sha256: item.sha256, byteLength: item.byteLength, role, input: { kind: "derivative2", receipt: derivative2.receipt, selector: derivative2.selector } });
      continue;
    }
    const exactBytes = assembledBytes.get(item.path) ?? input.artifactBytes[item.path];
    if (!exactBytes || hash(exactBytes) !== item.sha256 || exactBytes.byteLength !== item.byteLength) throw new Error(`Record staging requires exact bytes for ${item.path}`);
    const target = join(work, item.path);
    await ensurePrivateDirectoryPath(dirname(target), work);
    await writeExclusiveOrMatch(target, exactBytes);
    const file = await sealedFile(target, work, exactBytes);
    entries.push({ relativePath: item.path, sha256: item.sha256, byteLength: item.byteLength, role, input: { kind: "assembled", file } });
  }
  const plan: PublicationPlan = { schema: "apparent.bundle-publication/1", publicationId: input.publicationId, materials: structuredClone(materials.manifest.key), bundleId: materials.payload.bundleId, stagingRoot: work, destination: { path: destination, mode: "absent" }, artifactVersion: 1, entries, manifestPath: "apparent.json", summaryPath: "summary.md", profile: "apparent-bundle/1" };
  const planBytes = new TextEncoder().encode(`${JSON.stringify(plan, null, 2)}\n`);
  const planPath = join(work, "publication-plan.json");
  await writeExclusiveOrMatch(planPath, planBytes);
  const planFile = await sealedFile(planPath, work, planBytes);
  const envelope: PublishEnvelope = { stagingRoot: work, planFile, planSha256: planFile.sha256 };
  const publishInput: CapturePublishInput = { ownerToken: input.ownerToken, captureId: materials.manifest.key.captureId, leaseId: materials.payload.leaseId, publicationId: input.publicationId, planFile: envelope, planBytes, materials, ...(input.derivatives ? { derivatives: input.derivatives } : {}), ...(input.derivatives2 ? { derivatives2: input.derivatives2 } : {}) };
  const checked = preflightCapturePublication(publishInput);
  if (checked.plan.publicationId !== input.publicationId) throw new Error("SDK publication preflight changed publication identity");
  try { await lstat(destination); throw new Error("Record publication destination appeared during staging"); }
  catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
  return { input: publishInput, plan: checked.plan, inventory: input.assembly.inventory };
}

function derivative2Fingerprint(documents: readonly Derivative2DocumentInput[] | undefined, bindings: Derivative2ArtifactBinding | undefined): unknown {
  return canonicalDerivative2Keys({ documents: documents?.map(document => ({ envelope: document.envelope, bytesSha256: hash(document.bytes), request: document.request,
    layoutBytesSha256: document.layoutBytes ? hash(document.layoutBytes) : null })) ?? null, bindings: bindings ?? null });
}

/** JSON object order is not part of receipt/selector identity; exact detached byte hashes remain bound. */
function canonicalDerivative2Keys<T>(value: T): T {
  if (Array.isArray(value)) return value.map(canonicalDerivative2Keys) as T;
  if (value !== null && typeof value === "object") return Object.fromEntries(Object.entries(value)
    .sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0).map(([key, item]) => [key, canonicalDerivative2Keys(item)])) as T;
  return value;
}

async function ensurePrivateDirectory(path: string): Promise<void> {
  const parts = path.slice(1).split("/"); let current = "/";
  for (const part of parts) { const parent = current; current = join(current, part); try { await mkdir(current, { mode: privateMode }); await syncDirectory(parent); } catch (error) { if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw new Error("Record staging root is unavailable"); }
    const info = await lstat(current); if (!info.isDirectory() || info.isSymbolicLink()) throw new Error("Record staging root contains an alias");
    if (current === path && (info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0)) throw new Error("Record staging root is not private"); }
  if (await realpath(path) !== path) throw new Error("Record staging root is aliased");
}
async function assertDestinationParents(destination:string):Promise<void> {
  let current="/";
  for(const part of dirname(destination).slice(1).split("/").filter(Boolean)) {
    current=join(current,part);
    let info;
    try {info=await lstat(current);}
    catch(error){if((error as NodeJS.ErrnoException).code==="ENOENT")return;throw error;}
    if(!info.isDirectory()||info.isSymbolicLink())throw new Error("Record destination ancestor is not a directory or contains an alias");
  }
}
async function ensurePrivateDirectoryPath(path: string, root: string): Promise<void> {
  if (path !== root && !path.startsWith(`${root}/`)) throw new Error("Record staging path escapes its namespace");
  const parts = path.slice(root.length + 1).split("/"); let current = root;
  for (const part of parts) { const parent = current; current = join(current, part); try { await mkdir(current, { mode: privateMode }); await syncDirectory(parent); } catch (error) { if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw new Error("Record staging directory is unavailable"); }
    const info = await lstat(current); if (!info.isDirectory() || info.isSymbolicLink() || info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0) throw new Error("Record staging path contains a foreign directory or alias"); }
}
async function assertKnownStagingContents(root: string, files: ReadonlySet<string>): Promise<void> {
  const allowed = new Set<string>(files);
  for (const file of files) { let parent = dirname(file); while (parent !== ".") { allowed.add(parent); parent = dirname(parent); } }
  const visit = async (directory: string, prefix = ""): Promise<void> => {
    for (const name of await readdir(directory)) {
      const relative = prefix ? `${prefix}/${name}` : name;
      if (!allowed.has(relative)) throw new Error("Record staging namespace contains unrelated state; preserving it for reconciliation");
      const info = await lstat(join(directory, name));
      if (info.isSymbolicLink() || info.uid !== process.getuid?.() || (info.mode & 0o077) !== 0) throw new Error("Record staging namespace contains a foreign file or alias");
      if (info.isDirectory()) await visit(join(directory, name), relative);
      else if (!info.isFile() || info.nlink !== 1) throw new Error("Record staging namespace contains an aliased file");
    }
  };
  await visit(root);
}
async function writeExclusiveOrMatch(path: string, bytes: Uint8Array): Promise<void> {
  try { const handle = await open(path, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | (constants.O_NOFOLLOW ?? 0), 0o600); try { await handle.writeFile(bytes); await handle.sync(); } finally { await handle.close(); } await syncDirectory(dirname(path)); }
  catch (error) { if ((error as NodeJS.ErrnoException).code !== "EEXIST") throw new Error("Record staging write is incomplete; preserve the namespace for reconciliation");
    const existing = await readPrivateFile(path,true); if (!existing.bytes.equals(Buffer.from(bytes))) throw new Error("Record staging replay conflicts with existing bytes"); await syncDirectory(dirname(path)); }
}
async function sealedFile(path: string, root: string, bytes: Uint8Array): Promise<SealedFile> {
  const {info,bytes:actual}=await readPrivateFile(path);
  if (!actual.equals(Buffer.from(bytes))) throw new Error("Record staging bytes changed before sealing");
  return { relativePath: path.slice(root.length + 1), identity: { device: String(info.dev), inode: String(info.ino), size: Number(info.size), modifiedNs: String(info.mtimeNs) }, sha256: hash(actual) };
}
async function readPrivateFile(path:string,sync=false):Promise<{info:BigIntStats;bytes:Buffer}> {
  const handle=await open(path,(sync?constants.O_RDWR:constants.O_RDONLY)|(constants.O_NOFOLLOW??0));
  try {
    const before=await handle.stat({bigint:true});
    if(!before.isFile()||before.nlink!==1n||Number(before.uid)!==process.getuid?.()||(before.mode&0o077n)!==0n)throw new Error("Record staging file is not privately owned");
    const bytes=await handle.readFile();
    const after=await handle.stat({bigint:true}),named=await lstat(path,{bigint:true});
    if(after.nlink!==1n||after.size!==BigInt(bytes.length)||before.size!==after.size||before.mtimeNs!==after.mtimeNs||before.ctimeNs!==after.ctimeNs||named.isSymbolicLink()||named.dev!==after.dev||named.ino!==after.ino)throw new Error("Record staging file changed while sealing");
    if(sync)await handle.sync();
    return {info:after,bytes};
  } finally {await handle.close();}
}
async function syncDirectory(path: string): Promise<void> {
  const handle = await open(path, constants.O_RDONLY | (constants.O_DIRECTORY ?? 0) | (constants.O_NOFOLLOW ?? 0));
  try { await handle.sync(); } finally { await handle.close(); }
}
