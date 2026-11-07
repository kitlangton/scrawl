export type RuntimeFile = { path: string; mode: number; sha256: string };

export type NativeManifest = {
  schema: 1;
  sourceSha: string;
  target: 'linux-x64';
  rustTriple: 'x86_64-unknown-linux-gnu';
  capabilities: ['media/1'] | ['media/1', 'media/2'];
  visualObservations?: {
    schema: 'apparent.frame-observation-profile/1';
    appliesTo: 'media/2-frame-png';
    chunkType: 'apOB';
    algorithm: 'apparent.visual-luma/1';
  };
  media2Launch?: {
    argv: ['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '2'];
    sharesCaptureRuntime: true;
  };
  recoveryLaunch?: {
    schema: 'apparent.engine-recovery-peer/1';
    channel: 'media/2';
    argv: ['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '2', '--recovery-peer-v1', '<private-peer-nonce>'];
    startsFreshCapture: false;
    onInputEof: 'retain-admitted-finite-work';
  };
  launch: {
    entry: 'apparent-engine-frames';
    argv: ['--media-runtime-directory', '<private-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '1'];
    worker: 'libexec/apparent-engine-frames';
    pathLookup: false;
    runtimeBuilds: false;
  };
  environment: Record<string, unknown>;
  runtimeFiles: RuntimeFile[];
};

export type CaptureManifest = {
  schema: 1;
  sourceSha: string;
  recoveryLaunch: {
    schema: 'apparent.engine-recovery-peer/1';
    channel: 'capture/2';
    argv: ['--runtime-directory', '<private-runtime-dir>', '--capture-channel', 'capture', '--capture-protocol-version', '2', '--recovery-peer-v1', '<private-peer-nonce>'];
    startsFreshCapture: false;
    onInputEof: 'retain-admitted-finite-work';
  };
  eligibleForReuse: true;
  target: 'linux-x64';
  rustTriple: 'x86_64-unknown-linux-gnu';
  capabilities: string[];
  launch: {
    entry: 'apparent-engine-capture-launch';
    argv: ['--runtime-directory', '<private-runtime-dir>', '--capture-channel', 'capture', '--capture-protocol-version', '2'];
    worker: 'libexec/apparent-engine-capture';
    pathLookup: false;
    runtimeBuilds: false;
    shutdownTimeoutMs: 40000;
  };
  environment: Record<string, unknown>;
  runtimeFiles: RuntimeFile[];
};

export type InstallerInput = {
  agentPath: string;
  agentSha256: string;
  nativeRoot: string;
  manifestSha256: string;
  prefix: string;
  captureRoot?: string;
  captureManifestSha256?: string;
};

export type InventoryLimits = {
  maxFiles: number;
  maxEntries: number;
  maxFileBytes: number;
  maxTotalBytes: number;
  maxPathBytes: number;
};

export type HandshakeResult = {
  reply: {
    channel: string;
    protocolVersion: number;
    implementation: string;
    capabilities?: { decodeFrames?: boolean };
  };
  closeOutcome: string;
};

/** Injectable only through the internal TypeScript API; the production CLI has no test-peer option. */
export type InstallerDependencies = {
  installNpm(snapshotPath: string, installRoot: string, privateHome: string): void;
  smokeCli(cliPath: string, privateHome: string, installRoot: string): void;
  handshake(sdkMediaModule: string, entry: string, runtime: string): Promise<HandshakeResult>;
  captureHandshake?(entry: string, runtime: string, shutdownTimeoutMs: number): Promise<{
    reply: { channel: string; protocolVersion: number; implementation: string; readiness?: string };
    closeOutcome: string;
  }>;
};

export const DEFAULT_LIMITS: InventoryLimits = {
  maxFiles: 4096,
  maxEntries: 8192,
  maxFileBytes: 256_000_000,
  maxTotalBytes: 1_500_000_000,
  maxPathBytes: 1024,
};

export const MAX_AGENT_ARCHIVE_BYTES = 1_500_000_000;
export const MAX_MANIFEST_BYTES = 16 * 1024 * 1024;
