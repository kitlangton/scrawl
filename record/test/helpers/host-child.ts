import { SqliteCaptureMappingStore } from "../../dist/backend/host/capture-mapping-store.js";

const [root, requestId, captureId, ownerToken] = process.argv.slice(2);
if (!root || !requestId || !captureId || !ownerToken) throw new Error("missing child arguments");
const result = await new SqliteCaptureMappingStore(root).putIfAbsent({
  requestId,
  captureId,
  ownerToken,
  bundleId: `bundle-${captureId}`,
  bundlePath: `/private/${captureId}`,
});
process.stdout.write(JSON.stringify(result));
if (process.argv[6] === "lose-reply") process.exit(17);
