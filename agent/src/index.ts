/** Agent application APIs; importing this module never starts a command or Engine. */
export { resolveBundle, resolveCaptures, readManifest, readBundle } from './bundle.js';
export { markEntries, markLine } from './inspection.js';
export { recordWithEngine, type RecordingClient } from './recording.js';
export { parseClockMs, selectFrameTimes, decodeFrames, type FramesOptions, type FrameSelection } from './frames.js';
