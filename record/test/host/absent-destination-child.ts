import { AbsentDestinationAllocator } from "../../dist/backend/host/index.js";

const [controlRoot, destinationRoot, requestId] = process.argv.slice(2);
const binding = await new AbsentDestinationAllocator(controlRoot!, destinationRoot!).allocate(requestId!);
if (process.env.RECORD_ALLOCATOR_LOSE_REPLY === "1") process.exit(0);
process.stdout.write(JSON.stringify(binding));
