import { AbsentDestinationAllocator } from "../../dist/backend/host/index.js";

const [controlRoot, destinationRoot, requestId, exactPath] = process.argv.slice(2);
const allocator = new AbsentDestinationAllocator(controlRoot!, destinationRoot!);
const binding = exactPath ? await allocator.reserveExact(requestId!, exactPath) : await allocator.allocate(requestId!);
if (process.env.RECORD_ALLOCATOR_LOSE_REPLY === "1") process.exit(0);
process.stdout.write(JSON.stringify(binding));
