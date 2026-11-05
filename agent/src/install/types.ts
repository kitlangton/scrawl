export type RuntimeFile = { path: string; mode: number; sha256: string };

export type NativeManifest = {
  schema: 1;
  sourceSha: string;
  target: 'linux-x64';
  rustTriple: 'x86_64-unknown-linux-gnu';
  capabilities: ['media/1'];
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

export type InstallerInput = {
  agentPath: string;
  agentSha256: string;
  nativeRoot: string;
  manifestSha256: string;
  prefix: string;
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
