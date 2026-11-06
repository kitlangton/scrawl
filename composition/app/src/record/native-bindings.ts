import { createNativeHostPort } from '@apparent/native-host';
import { createNativePenPort } from '@apparent/native-host/pen';
import { createNativeClipboardPort, MAX_UTF8_TEXT_BYTES } from '@apparent/native-host/clipboard';
import {
  createRecordLocalClipboardPort,
  createRecordNativePort,
  createRecordPenConfigPort,
  type RecordLocalHandoffResolution,
} from '@apparent/record/host/native';
import { createRecordPenColorConfig } from '@apparent/config-host';

export interface RecordNativeBindingOptions {
  /** Permission policy is supplied by trusted Record composition, never a renderer. */
  requiredPermissionIds: readonly string[];
  /** Resolves only an authenticated, already-saved capture/artifact pair. */
  resolveHandoff(identity: Readonly<{ captureId: string; artifactId: string }>): Promise<RecordLocalHandoffResolution | null>;
  /** The admitted executable and config path for Record's sole existing writer. */
  config: { executable: string; executableSha256: string; configPath: string };
}

export interface RecordNativeBindings {
  native: ReturnType<typeof createRecordNativePort>;
  penConfig: ReturnType<typeof createRecordPenConfigPort>;
  clipboard: ReturnType<typeof createRecordLocalClipboardPort>;
  /** Closes only the config backend created by this binding. Safe to call repeatedly. */
  close(): Promise<void>;
}

/**
 * Binds the public native/config producers to Record's public owner adapters.
 * This is privileged main-process composition; no options originate in a renderer.
 */
export async function createRecordNativeBindings(options: RecordNativeBindingOptions): Promise<RecordNativeBindings> {
  if (!options || !Array.isArray(options.requiredPermissionIds)
    || options.requiredPermissionIds.some(id => typeof id !== 'string')
    || typeof options.resolveHandoff !== 'function' || !options.config
    || typeof options.config.executable !== 'string'
    || typeof options.config.executableSha256 !== 'string'
    || typeof options.config.configPath !== 'string') {
    throw new TypeError('Invalid Record native binding options');
  }

  // Snapshot all caller-controlled values before config setup yields.
  const requiredPermissionIds = Object.freeze([...options.requiredPermissionIds]);
  const resolveHandoff = options.resolveHandoff;
  const configOptions = Object.freeze({
    executable: options.config.executable,
    executableSha256: options.config.executableSha256,
    configPath: options.config.configPath,
  });

  const nativeHost = createNativeHostPort();
  const nativePen = createNativePenPort();
  const clipboardProducer = createNativeClipboardPort();

  let config: Awaited<ReturnType<typeof createRecordPenColorConfig>> | undefined;
  try {
    config = await createRecordPenColorConfig(configOptions);
    const native = createRecordNativePort(nativeHost, { requiredPermissionIds });
    const penConfig = createRecordPenConfigPort({ config, nativePen, nativeHost });
    const clipboard = createRecordLocalClipboardPort({
      clipboard: clipboardProducer,
      nativeHost,
      resolveHandoff,
      maximumTextBytes: MAX_UTF8_TEXT_BYTES,
    });

    let closing: Promise<void> | undefined;
    return {
      native,
      penConfig,
      clipboard,
      close() {
        closing ??= Promise.resolve().then(() => config!.close());
        return closing;
      },
    };
  } catch (error) {
    if (config) {
      try { await config.close(); }
      catch (closeError) {
        throw new AggregateError([error, closeError], 'Record native binding setup and config cleanup failed');
      }
    }
    throw error;
  }
}

