import { createNativeHostPort } from '@apparent/native-host';
import { createNativeClipboardPort, MAX_SERIALIZED_FRAME_BYTES, MAX_UTF8_TEXT_BYTES } from '@apparent/native-host/clipboard';
import { createNativePenPort } from '@apparent/native-host/pen';
import type {
  NativeClipboardPort,
  NativeClipboardWriteResult,
  NativeHostPort,
  NativeHostSnapshot,
  NativePenPort,
  NativePenResult,
  NativePenSnapshot,
} from '@apparent/app-contracts/native';

const host: NativeHostPort = createNativeHostPort();
const pen: NativePenPort = createNativePenPort();
const clipboard: NativeClipboardPort = createNativeClipboardPort();
const limits: readonly number[] = [MAX_UTF8_TEXT_BYTES, MAX_SERIALIZED_FRAME_BYTES];

async function consumerContract(): Promise<NativeClipboardWriteResult> {
  const snapshot: NativeHostSnapshot = await host.readSnapshot();
  const penSnapshot: NativePenSnapshot = await pen.readStrokeColor();
  const penResult: NativePenResult = await pen.applyStrokeColor(null, { present: false, value: null }, {
    processEpoch: penSnapshot.processEpoch,
    revision: snapshot.revision,
  });
  void penResult;
  void limits;
  return clipboard.writeText('strict consumer', { processEpoch: snapshot.processEpoch, revision: snapshot.revision });
}

void consumerContract;
