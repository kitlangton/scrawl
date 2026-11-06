import type { NativeHostPort } from '@apparent/app-contracts/host';
import { createRecordNativePort } from '../../src/backend/host/native/index.ts';
declare const publicProducerPort: NativeHostPort;
// Compile-time conformance against the actual published producer API.
createRecordNativePort(publicProducerPort, { requiredPermissionIds: ['screen-recording'] });
