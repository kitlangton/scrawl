export type CursorPoint = Readonly<{ t: number; x: number; y: number }>

// A 4:5 phone viewport retains the entire height of the original recording.
const VISIBLE_WIDTH = (4 / 5) / (1916 / 1164)
const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, value))

function targetOffsetAt(time: number, points: readonly CursorPoint[]): number {
  if (!points.length) return 0
  const next = points.findIndex(point => point.t > time)
  const from = points[next < 0 ? points.length - 1 : Math.max(0, next - 1)]
  const to = points[next < 0 ? points.length - 1 : next]
  const progress = to.t > from.t ? clamp((time - from.t) / (to.t - from.t), 0, 1) : 0
  const eased = progress * progress * (3 - 2 * progress)
  const x = from.x + (to.x - from.x) * eased
  return clamp(x - VISIBLE_WIDTH / 2, 0, 1 - VISIBLE_WIDTH)
}

/** Spread quick cursor moves over three seconds without accumulating playback lag.
 * Sampling the media-clock path also makes replay and arbitrary seeks deterministic. */
export function cameraOffsetAt(time: number, points: readonly CursorPoint[]): number {
  const samples = 24
  let total = 0
  for (let index = 0; index < samples; index++)
    total += targetOffsetAt(time - 1.5 + (index + .5) * 3 / samples, points)
  return clamp(total / samples, 0, 1 - VISIBLE_WIDTH)
}

/** Use the media clock, so seeking, replay and buffering cannot desync the camera. */
export function followCursor(video: HTMLVideoElement, points: readonly CursorPoint[]): () => void {
  let frame: number | null = null
  const videoFrames = typeof video.requestVideoFrameCallback === "function"
  const paint = (time = video.currentTime) => video.style.setProperty("--cursor-offset", String(cameraOffsetAt(time, points)))
  const stop = () => {
    if (frame === null) return
    if (videoFrames) video.cancelVideoFrameCallback(frame)
    else cancelAnimationFrame(frame)
    frame = null
  }
  const schedule = () => {
    if (frame !== null || video.paused || video.ended) return
    if (videoFrames) frame = video.requestVideoFrameCallback((_now, metadata) => {
      frame = null
      paint(metadata.mediaTime)
      schedule()
    })
    else frame = requestAnimationFrame(() => { frame = null; paint(); schedule() })
  }
  const sync = () => paint()
  const play = () => { sync(); schedule() }
  const pause = () => { stop(); sync() }
  const events = { play, pause, ended: pause, seeking: sync, seeked: sync, loadedmetadata: sync, timeupdate: sync }
  for (const [name, listener] of Object.entries(events)) video.addEventListener(name, listener)
  sync()
  schedule()
  return () => {
    stop()
    for (const [name, listener] of Object.entries(events)) video.removeEventListener(name, listener)
    video.style.removeProperty("--cursor-offset")
  }
}
