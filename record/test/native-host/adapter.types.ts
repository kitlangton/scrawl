import type { NativeHostPort } from '@apparent/app-contracts/host';
import { createRecordNativePort } from '../../src/backend/host/native/index.ts';
declare const publicProducerPort: NativeHostPort;
// Compile-time conformance against the actual published producer API.
createRecordNativePort(publicProducerPort, { requiredPermissionIds: ['screen-recording'] });

import type { NativeClipboardPort } from '@apparent/app-contracts/host';
import { MAX_UTF8_TEXT_BYTES } from '@apparent/app-contracts/host';
import { createRecordLocalClipboardPort, type RecordLocalClipboardPortInput } from '../../src/backend/host/native/index.ts';
import type { RecordClipboardPort } from '../../src/capabilities/index.ts';
declare const clipboardProducer: NativeClipboardPort;
declare const resolveHandoff: RecordLocalClipboardPortInput['resolveHandoff'];
const confirmedCopy = createRecordLocalClipboardPort({ clipboard: clipboardProducer, nativeHost: publicProducerPort, resolveHandoff, maximumTextBytes: MAX_UTF8_TEXT_BYTES });
const providerCopy: RecordClipboardPort = confirmedCopy;
void providerCopy;
