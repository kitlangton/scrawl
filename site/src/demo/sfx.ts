// Cuelume's synthesizer (cuelume/src/audio/engine.ts and sounds/recipes.ts), trimmed to the
// sounds the demo uses, plus a continuous pen/drag friction voice and WebAudio narration playback.
import { type Mode, NARRATION, T } from "./script"

type BaseLayer = { offset?: number; attack: number; decay: number; peak: number }
type ToneLayer = BaseLayer & { kind: "tone"; waveform: OscillatorType; frequency: number; detune?: number; glideTo?: number; glideTime?: number }
type NoiseLayer = BaseLayer & { kind: "noise"; filterType: BiquadFilterType; filterFrequency: number; filterQ?: number }
type Shimmer = { delay: number; feedback: number; wet: number; lowpass: number }
type Recipe = { masterGain: number; layers: (ToneLayer | NoiseLayer)[]; shimmer?: Shimmer }

const RECIPES = {
  start: {
    masterGain: 0.54,
    layers: [
      { kind: "noise", filterType: "bandpass", filterFrequency: 2800, filterQ: 1.8, attack: 0.001, decay: 0.018, peak: 0.09 },
      { kind: "tone", waveform: "triangle", frequency: 523.25, attack: 0.005, decay: 0.16, peak: 0.075 },
      { kind: "tone", waveform: "sine", frequency: 783.99, offset: 0.045, attack: 0.006, decay: 0.2, peak: 0.085 },
      { kind: "tone", waveform: "sine", frequency: 1046.5, offset: 0.095, attack: 0.008, decay: 0.26, peak: 0.07 },
    ],
    shimmer: { delay: 0.11, feedback: 0.22, wet: 0.16, lowpass: 3800 },
  },
  chime: {
    masterGain: 0.5,
    layers: [
      { kind: "tone", waveform: "sine", frequency: 1046.5, attack: 0.006, decay: 0.22, peak: 0.09 },
      { kind: "tone", waveform: "sine", frequency: 1568, offset: 0.09, attack: 0.006, decay: 0.26, peak: 0.08 },
    ],
    shimmer: { delay: 0.12, feedback: 0.25, wet: 0.18, lowpass: 4000 },
  },
  droplet: {
    masterGain: 0.55,
    layers: [{ kind: "tone", waveform: "sine", frequency: 1200, glideTo: 550, glideTime: 0.14, attack: 0.004, decay: 0.2, peak: 0.075 }],
    shimmer: { delay: 0.09, feedback: 0.2, wet: 0.15, lowpass: 3000 },
  },
  whisper: {
    masterGain: 0.48,
    layers: [
      { kind: "noise", filterType: "lowpass", filterFrequency: 1600, filterQ: 0.7, attack: 0.025, decay: 0.13, peak: 0.04 },
      { kind: "tone", waveform: "sine", frequency: 880, glideTo: 660, glideTime: 0.14, offset: 0.01, attack: 0.012, decay: 0.14, peak: 0.025 },
    ],
  },
  tick: {
    masterGain: 0.4,
    layers: [
      { kind: "noise", filterType: "bandpass", filterFrequency: 5400, filterQ: 1.8, attack: 0.001, decay: 0.018, peak: 0.14 },
      { kind: "tone", waveform: "sine", frequency: 2600, attack: 0.001, decay: 0.012, peak: 0.018 },
    ],
  },
  press: {
    masterGain: 0.4,
    layers: [{ kind: "noise", filterType: "bandpass", filterFrequency: 1700, filterQ: 1.4, attack: 0.001, decay: 0.02, peak: 0.13 }],
  },
  release: {
    masterGain: 0.4,
    layers: [
      { kind: "noise", filterType: "bandpass", filterFrequency: 4600, filterQ: 1.8, attack: 0.001, decay: 0.016, peak: 0.12 },
      { kind: "tone", waveform: "sine", frequency: 3200, offset: 0.006, attack: 0.001, decay: 0.05, peak: 0.02 },
    ],
  },
  toggle: {
    masterGain: 0.4,
    layers: [
      { kind: "noise", filterType: "bandpass", filterFrequency: 2200, filterQ: 1.6, attack: 0.001, decay: 0.016, peak: 0.12 },
      { kind: "noise", filterType: "bandpass", filterFrequency: 3800, filterQ: 1.6, offset: 0.024, attack: 0.001, decay: 0.02, peak: 0.1 },
    ],
  },
  success: {
    masterGain: 0.5,
    layers: [
      { kind: "tone", waveform: "sine", frequency: 880, attack: 0.004, decay: 0.09, peak: 0.06 },
      { kind: "tone", waveform: "sine", frequency: 1108.73, offset: 0.06, attack: 0.004, decay: 0.1, peak: 0.06 },
      { kind: "tone", waveform: "sine", frequency: 1318.51, offset: 0.12, attack: 0.004, decay: 0.18, peak: 0.07 },
    ],
    shimmer: { delay: 0.1, feedback: 0.22, wet: 0.16, lowpass: 4500 },
  },
  pulse: {
    masterGain: 0.42,
    layers: [
      { kind: "noise", filterType: "bandpass", filterFrequency: 2600, filterQ: 2.4, attack: 0.001, decay: 0.022, peak: 0.08 },
      { kind: "tone", waveform: "triangle", frequency: 620, glideTo: 1240, glideTime: 0.07, attack: 0.002, decay: 0.085, peak: 0.055 },
    ],
  },
} as const satisfies Record<string, Recipe>

export type Sound = keyof typeof RECIPES

const SOURCE_STOP_PADDING = 0.05
const CLEANUP_MARGIN = 0.05
const INAUDIBLE_GAIN = 0.001
const OUTPUT_GAIN = 4
// Effects sit under the narration.
const SFX_VOLUME = 0.6

let ctx: AudioContext | null = null
let output: GainNode | null = null
let narrationGain: GainNode | null = null
let enabled = true
let unlockedOnce = false
let listenersBound = false

const playedHistory: { sound: Sound; at: number }[] = []
let startSoundCount = 0
let lastTimelineMs = 0

function syncAudioSession(play: boolean) {
  if (typeof navigator === "undefined") return
  try {
    const session = (navigator as unknown as { audioSession?: { type: string } }).audioSession
    if (session) session.type = play ? "playback" : "ambient"
  } catch {
    // Ignore on browsers without navigator.audioSession
  }
}

function context(): AudioContext | null {
  if (ctx) return ctx
  if (typeof window === "undefined") return null
  const Ctor = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
  if (!Ctor) return null
  try {
    ctx = new Ctor()
  } catch {
    return null
  }
  if (!listenersBound) {
    listenersBound = true
    const resumeIfVisible = () => {
      if (!document.hidden && ctx && ctx.state === "suspended" && unlockedOnce) {
        ctx.resume().catch(() => {})
      }
    }
    window.addEventListener("pageshow", resumeIfVisible)
    document.addEventListener("visibilitychange", resumeIfVisible)
  }
  return ctx
}

function out(c: AudioContext): GainNode {
  if (output) return output
  output = c.createGain()
  output.gain.value = OUTPUT_GAIN * SFX_VOLUME
  const limiter = c.createDynamicsCompressor()
  limiter.threshold.value = -8
  limiter.knee.value = 6
  limiter.ratio.value = 12
  limiter.attack.value = 0.002
  limiter.release.value = 0.08
  output.connect(limiter).connect(c.destination)
  return output
}

function narrationOut(c: AudioContext): GainNode {
  if (narrationGain) return narrationGain
  narrationGain = c.createGain()
  narrationGain.gain.value = 1
  narrationGain.connect(c.destination)
  return narrationGain
}

// Must run inside a user gesture: creates or resumes the shared context and primes WebKit audio.
export function unlock() {
  syncAudioSession(enabled)
  const c = context()
  if (!c) return
  narrationOut(c)
  if (c.state !== "running") {
    c.resume().catch(() => {})
  }
  if (!unlockedOnce) {
    unlockedOnce = true
    try {
      // Starting a 1-sample buffer synchronously inside the user tap unlocks iOS WebKit's audio thread.
      const silent = c.createBuffer(1, 1, 22050)
      const src = c.createBufferSource()
      src.buffer = silent
      src.connect(c.destination)
      src.start(0)
    } catch {
      // Ignore
    }
  }
  preloadNarration()
}

export function setEnabled(v: boolean) {
  enabled = v
  syncAudioSession(v)
  if (!v) {
    for (const ch of channels.keys()) scribbleStop(ch)
    stopNarration()
    if (narrationGain && ctx) {
      narrationGain.gain.cancelScheduledValues(ctx.currentTime)
      narrationGain.gain.setValueAtTime(0, ctx.currentTime)
    }
  } else if (narrationGain && ctx) {
    narrationGain.gain.cancelScheduledValues(ctx.currentTime)
    narrationGain.gain.setValueAtTime(1, ctx.currentTime)
  }
}

// Allow scheduling immediately after unlock() even while ctx.resume() is transitioning from "suspended" to "running".
function activeContext(): AudioContext | null {
  if (!enabled || !ctx) return null
  if (ctx.state === "suspended" && unlockedOnce) {
    ctx.resume().catch(() => {})
  }
  return ctx.state === "closed" ? null : ctx
}

function noiseBuffer(c: AudioContext, seconds: number) {
  const length = Math.max(1, Math.floor(seconds * c.sampleRate))
  const buffer = c.createBuffer(1, length, c.sampleRate)
  const data = buffer.getChannelData(0)
  for (let i = 0; i < length; i++) data[i] = 2 * Math.random() - 1
  return buffer
}

function renderTone(c: AudioContext, dest: AudioNode, l: ToneLayer, start: number) {
  const osc = c.createOscillator()
  osc.type = l.waveform
  osc.frequency.setValueAtTime(l.frequency, start)
  if (l.detune) osc.detune.value = l.detune
  if (l.glideTo !== undefined) osc.frequency.exponentialRampToValueAtTime(l.glideTo, start + (l.glideTime ?? l.attack + l.decay))
  const gain = c.createGain()
  gain.gain.setValueAtTime(0.0001, start)
  gain.gain.exponentialRampToValueAtTime(l.peak, start + l.attack)
  gain.gain.exponentialRampToValueAtTime(0.0001, start + l.attack + l.decay)
  osc.connect(gain).connect(dest)
  osc.start(start)
  osc.stop(start + l.attack + l.decay + SOURCE_STOP_PADDING)
}

function renderNoise(c: AudioContext, dest: AudioNode, l: NoiseLayer, start: number) {
  const duration = l.attack + l.decay + SOURCE_STOP_PADDING
  const source = c.createBufferSource()
  source.buffer = noiseBuffer(c, duration)
  const filter = c.createBiquadFilter()
  filter.type = l.filterType
  filter.frequency.value = l.filterFrequency
  if (l.filterQ !== undefined) filter.Q.value = l.filterQ
  const gain = c.createGain()
  gain.gain.setValueAtTime(0.0001, start)
  gain.gain.exponentialRampToValueAtTime(l.peak, start + l.attack)
  gain.gain.exponentialRampToValueAtTime(0.0001, start + l.attack + l.decay)
  source.connect(filter).connect(gain).connect(dest)
  source.start(start)
  source.stop(start + duration)
}

function attachShimmer(c: AudioContext, source: AudioNode, dest: AudioNode, s: Shimmer): AudioNode[] {
  const delay = c.createDelay(1)
  delay.delayTime.value = s.delay
  const filter = c.createBiquadFilter()
  filter.type = "lowpass"
  filter.frequency.value = s.lowpass
  const feedback = c.createGain()
  feedback.gain.value = s.feedback
  const wet = c.createGain()
  wet.gain.value = s.wet
  source.connect(delay)
  delay.connect(filter)
  filter.connect(feedback)
  feedback.connect(delay)
  filter.connect(wet)
  wet.connect(dest)
  return [delay, filter, feedback, wet]
}

export function play(sound: Sound, volume = 1, delay = 0) {
  const c = activeContext()
  if (!c) return
  if (sound === "start") startSoundCount++
  playedHistory.push({ sound, at: performance.now() })
  if (playedHistory.length > 64) playedHistory.shift()
  const recipe: Recipe = RECIPES[sound]
  const now = c.currentTime + delay
  const dest = out(c)
  const master = c.createGain()
  master.gain.value = recipe.masterGain * volume
  master.connect(dest)
  const shimmer = recipe.shimmer ? attachShimmer(c, master, dest, recipe.shimmer) : []
  for (const l of recipe.layers) {
    const start = now + (l.offset ?? 0)
    if (l.kind === "tone") renderTone(c, master, l, start)
    else renderNoise(c, master, l, start)
  }
  const end = Math.max(...recipe.layers.map((l) => (l.offset ?? 0) + l.attack + l.decay + SOURCE_STOP_PADDING))
  const s = recipe.shimmer
  const tail = s && s.feedback > 0 ? s.delay * (1 + Math.ceil(Math.log(INAUDIBLE_GAIN) / Math.log(s.feedback))) : 0
  setTimeout(() => {
    master.disconnect()
    for (const n of shimmer) n.disconnect()
  }, (delay + end + tail + CLEANUP_MARGIN) * 1000)
}

// ---------- Continuous friction: a dragged selection or a marker on glass ----------

export type Texture = "glass" | "marker"

const TEXTURE = {
  // A soft, low-passed slide.
  glass: { type: "lowpass" as BiquadFilterType, lo: 600, hi: 1200, q: 0.8, gain: 0.07, hum: 0.008, humLo: 90, humHi: 140 },
  // A felt tip squeaking across glass.
  marker: { type: "bandpass" as BiquadFilterType, lo: 1400, hi: 2800, q: 1.3, gain: 0.16, hum: 0.012, humLo: 170, humHi: 260 },
}

type Voice = { texture: Texture; noise: AudioBufferSourceNode; filter: BiquadFilterNode; gain: GainNode; hum: OscillatorNode; humGain: GainNode }

const channels = new Map<string, Voice>()
let pink: AudioBuffer | null = null

function pinkBuffer(c: AudioContext) {
  if (pink && pink.sampleRate === c.sampleRate) return pink
  const length = c.sampleRate * 2
  pink = c.createBuffer(1, length, c.sampleRate)
  const d = pink.getChannelData(0)
  let b0 = 0, b1 = 0, b2 = 0, b3 = 0, b4 = 0, b5 = 0, b6 = 0
  for (let i = 0; i < length; i++) {
    const w = Math.random() * 2 - 1
    b0 = 0.99886 * b0 + w * 0.0555179
    b1 = 0.99332 * b1 + w * 0.0750759
    b2 = 0.969 * b2 + w * 0.153852
    b3 = 0.8665 * b3 + w * 0.3104856
    b4 = 0.55 * b4 + w * 0.5329522
    b5 = -0.7616 * b5 - w * 0.016898
    d[i] = (b0 + b1 + b2 + b3 + b4 + b5 + b6 + w * 0.5362) * 0.11
    b6 = w * 0.115926
  }
  return pink
}

// Moves a param toward `value`, replacing anything scheduled after now.
function glide(p: AudioParam, value: number, now: number, tc: number) {
  const hold = (p as AudioParam & { cancelAndHoldAtTime?: (t: number) => void }).cancelAndHoldAtTime
  if (hold) hold.call(p, now)
  else {
    p.cancelScheduledValues(now)
    p.setValueAtTime(p.value, now)
  }
  p.setTargetAtTime(value, now, tc)
}

function voice(c: AudioContext, ch: string, texture: Texture): Voice {
  const existing = channels.get(ch)
  if (existing && existing.texture === texture) return existing
  if (existing) scribbleStop(ch)
  const cfg = TEXTURE[texture]
  const noise = c.createBufferSource()
  noise.buffer = pinkBuffer(c)
  noise.loop = true
  const filter = c.createBiquadFilter()
  filter.type = cfg.type
  filter.frequency.value = cfg.lo
  filter.Q.value = cfg.q
  const gain = c.createGain()
  gain.gain.value = 0
  const hum = c.createOscillator()
  hum.type = "sine"
  hum.frequency.value = cfg.humLo
  const humGain = c.createGain()
  humGain.gain.value = 0
  const dest = out(c)
  noise.connect(filter).connect(gain).connect(dest)
  hum.connect(humGain).connect(dest)
  noise.start(c.currentTime, Math.random() * 1.5)
  hum.start()
  const v = { texture, noise, filter, gain, hum, humGain }
  channels.set(ch, v)
  return v
}

// `speed` is 0..1. Each update also schedules a fade, so a pen that stops moving goes quiet.
export function scribble(ch: string, texture: Texture, speed: number) {
  const c = activeContext()
  if (!c || c.state !== "running") return
  const v = voice(c, ch, texture)
  const cfg = TEXTURE[texture]
  const s = Math.min(1, Math.max(0, speed))
  const now = c.currentTime
  const level = s < 0.02 ? 0 : Math.pow(s, 0.7)
  glide(v.gain.gain, cfg.gain * level, now, 0.025)
  v.gain.gain.setTargetAtTime(0, now + 0.12, 0.05)
  glide(v.filter.frequency, cfg.lo + (cfg.hi - cfg.lo) * s, now, 0.03)
  glide(v.humGain.gain, cfg.hum * level, now, 0.04)
  v.humGain.gain.setTargetAtTime(0, now + 0.12, 0.05)
  glide(v.hum.frequency, cfg.humLo + (cfg.humHi - cfg.humLo) * s, now, 0.05)
}

export function scribbleStop(ch: string) {
  const v = channels.get(ch)
  if (!v || !ctx) return
  channels.delete(ch)
  const now = ctx.currentTime
  glide(v.gain.gain, 0, now, 0.03)
  glide(v.humGain.gain, 0, now, 0.03)
  v.noise.stop(now + 0.25)
  v.hum.stop(now + 0.25)
  setTimeout(() => {
    v.gain.disconnect()
    v.humGain.disconnect()
  }, 400)
}

// ---------- Narration (WebAudio AudioBufferSourceNode locked to AudioContext clock) ----------

const rawNarration = new Map<Mode, Promise<ArrayBuffer | null>>()
const decodedNarration = new Map<Mode, AudioBuffer>()
const decodingNarration = new Map<Mode, Promise<AudioBuffer | null>>()

type ActiveNarration = {
  mode: Mode
  source: AudioBufferSourceNode
  nodeGain: GainNode
  startCtxTime: number
  offsetSec: number
  durationSec: number
  ended: boolean
}

let activeNarration: ActiveNarration | null = null

// Smoothly silences the voice model's direction-tag gaps directly in the PCM buffer with an 8ms cosine taper.
function applyMuteWindows(buffer: AudioBuffer, windows: [number, number][]) {
  const sr = buffer.sampleRate
  const fade = Math.max(1, Math.floor(0.008 * sr))
  for (let ch = 0; ch < buffer.numberOfChannels; ch++) {
    const data = buffer.getChannelData(ch)
    const len = data.length
    for (const [a, b] of windows) {
      const start = Math.max(0, Math.floor(a * sr))
      const end = Math.min(len, Math.ceil(b * sr))
      for (let i = start; i < end; i++) {
        if (i < start + fade) {
          const k = (i - start) / fade
          data[i] *= 0.5 * (1 + Math.cos(Math.PI * k))
        } else if (i > end - fade) {
          const k = (end - i) / fade
          data[i] *= 0.5 * (1 + Math.cos(Math.PI * k))
        } else {
          data[i] = 0
        }
      }
    }
  }
}

function fetchNarrationBytes(mode: Mode): Promise<ArrayBuffer | null> {
  const existing = rawNarration.get(mode)
  if (existing) return existing
  const p = fetch(NARRATION[mode].src)
    .then((r) => (r.ok ? r.arrayBuffer() : null))
    .catch(() => null)
  rawNarration.set(mode, p)
  return p
}

function decodeNarrationBuffer(mode: Mode): Promise<AudioBuffer | null> {
  const ready = decodedNarration.get(mode)
  if (ready) return Promise.resolve(ready)
  const inflight = decodingNarration.get(mode)
  if (inflight) return inflight

  const p = fetchNarrationBytes(mode).then(async (bytes) => {
    if (!bytes) return null
    try {
      const OfflineCtor =
        typeof window !== "undefined"
          ? window.OfflineAudioContext ?? (window as unknown as { webkitOfflineAudioContext?: typeof OfflineAudioContext }).webkitOfflineAudioContext
          : undefined
      const decoder: BaseAudioContext | null = ctx ?? (OfflineCtor ? new OfflineCtor(1, 1, 44100) : null)
      if (!decoder) return null
      const copy = bytes.slice(0)
      const buf = await new Promise<AudioBuffer>((resolve, reject) => {
        const res = decoder.decodeAudioData(
          copy,
          (b) => resolve(b),
          (e) => reject(e),
        )
        if (res && typeof res.then === "function") res.then(resolve, reject)
      })
      applyMuteWindows(buf, NARRATION[mode].mute)
      decodedNarration.set(mode, buf)
      return buf
    } catch {
      decodingNarration.delete(mode)
      return null
    }
  })

  decodingNarration.set(mode, p)
  return p
}

export function preloadNarration() {
  void decodeNarrationBuffer("mac")
  void decodeNarrationBuffer("iphone")
}

export function stopNarration() {
  const cur = activeNarration
  if (!cur) return
  activeNarration = null
  cur.ended = true
  try {
    if (ctx && ctx.state === "running") {
      const now = ctx.currentTime
      cur.nodeGain.gain.cancelScheduledValues(now)
      cur.nodeGain.gain.setValueAtTime(cur.nodeGain.gain.value, now)
      cur.nodeGain.gain.linearRampToValueAtTime(0.0001, now + 0.015)
      cur.source.stop(now + 0.018)
      setTimeout(() => {
        cur.source.disconnect()
        cur.nodeGain.disconnect()
      }, 40)
    } else {
      cur.source.stop()
      cur.source.disconnect()
      cur.nodeGain.disconnect()
    }
  } catch {
    // Ignore if already stopped
  }
}

function updateNarrationGain(c: AudioContext, mode: Mode, sec: number) {
  const windows = NARRATION[mode].mute
  const inMuteWindow = windows.some(([a, b]) => sec >= a && sec < b)
  const g = narrationOut(c)
  const target = !enabled || inMuteWindow ? 0 : 1
  g.gain.cancelScheduledValues(c.currentTime)
  g.gain.setValueAtTime(target, c.currentTime)
}

// Synchronizes WebAudio narration playback with the demo timeline.
// Returns the authoritative demo clock time in ms while narration is actively playing, or null otherwise.
export function syncNarration(mode: Mode, tMs: number, playing: boolean, sound: boolean): number | null {
  lastTimelineMs = tMs
  const endMs = NARRATION[mode].end
  const want = playing && sound && enabled && tMs >= T.record && tMs < endMs - 20
  if (!want) {
    if (activeNarration) stopNarration()
    if (narrationGain && ctx) {
      narrationGain.gain.cancelScheduledValues(ctx.currentTime)
      narrationGain.gain.setValueAtTime(sound && enabled ? 1 : 0, ctx.currentTime)
    }
    return null
  }

  const c = context()
  if (!c) return null
  if (c.state === "suspended" && unlockedOnce) {
    c.resume().catch(() => {})
  }

  const buf = decodedNarration.get(mode)
  if (!buf) {
    void decodeNarrationBuffer(mode)
    return null
  }

  if (c.state !== "running") return null

  const wantSec = Math.max(0, (tMs - T.record) / 1000)
  if (activeNarration && activeNarration.mode === mode && !activeNarration.ended) {
    const playheadSec = activeNarration.offsetSec + (c.currentTime - activeNarration.startCtxTime)
    if (Math.abs(playheadSec - wantSec) <= 0.12) {
      updateNarrationGain(c, mode, playheadSec)
      const masterMs = T.record + playheadSec * 1000
      lastTimelineMs = masterMs
      return masterMs
    }
    stopNarration()
  } else if (activeNarration) {
    stopNarration()
  }

  if (wantSec >= buf.duration - 0.02) return null

  const source = c.createBufferSource()
  source.buffer = buf
  const nodeGain = c.createGain()
  const now = c.currentTime
  if (wantSec > 0.02) {
    nodeGain.gain.setValueAtTime(0.0001, now)
    nodeGain.gain.linearRampToValueAtTime(1, now + 0.012)
  } else {
    nodeGain.gain.setValueAtTime(1, now)
  }
  source.connect(nodeGain).connect(narrationOut(c))

  const active: ActiveNarration = {
    mode,
    source,
    nodeGain,
    startCtxTime: now,
    offsetSec: wantSec,
    durationSec: buf.duration,
    ended: false,
  }
  source.onended = () => {
    active.ended = true
    if (activeNarration === active) activeNarration = null
  }
  source.start(now, wantSec)
  activeNarration = active
  updateNarrationGain(c, mode, wantSec)
  const masterMs = T.record + wantSec * 1000
  lastTimelineMs = masterMs
  return masterMs
}

export type AudioDiagnostics = {
  ctxState: AudioContextState | "uninitialized"
  ctxCurrentTime: number
  narrationPlaying: boolean
  narrationMode: Mode | null
  narrationCurrentTime: number
  narrationDecoded: { mac: boolean; iphone: boolean }
  narrationGain: number
  timelineMs: number
  driftMs: number
  startSoundCount: number
  playedSounds: { sound: Sound; at: number }[]
}

export function getDiagnostics(): AudioDiagnostics {
  const c = ctx
  const active = activeNarration && !activeNarration.ended && c ? activeNarration : null
  const narrationCurrentTime = active && c ? active.offsetSec + (c.currentTime - active.startCtxTime) : 0
  const expectedSec = Math.max(0, (lastTimelineMs - T.record) / 1000)
  const driftMs = active ? Math.abs(narrationCurrentTime - expectedSec) * 1000 : 0
  return {
    ctxState: c ? c.state : "uninitialized",
    ctxCurrentTime: c ? c.currentTime : 0,
    narrationPlaying: !!active,
    narrationMode: active ? active.mode : null,
    narrationCurrentTime,
    narrationDecoded: {
      mac: decodedNarration.has("mac"),
      iphone: decodedNarration.has("iphone"),
    },
    narrationGain: narrationGain ? narrationGain.gain.value : 1,
    timelineMs: lastTimelineMs,
    driftMs,
    startSoundCount,
    playedSounds: [...playedHistory],
  }
}

if (typeof window !== "undefined") {
  ;(window as unknown as { __scrawlAudio?: () => AudioDiagnostics }).__scrawlAudio = getDiagnostics
}

// ---------- Scripted cues ----------

export type Cue = { t: number; sound: Sound; volume?: number }

export function demoCues(mode: Mode): Cue[] {
  const keys = (down: number[], up: number[]): Cue[] => [
    ...down.map((t) => ({ t, sound: "press" as const })),
    ...up.map((t) => ({ t, sound: "release" as const })),
  ]
  const cues: Cue[] = [
    ...keys([T.keys, T.keys + 80, T.keys + 160], [T.keysUp, T.keysUp + 45, T.keysUp + 90]),
    { t: T.keysUp + 20, sound: "pulse", volume: 0.7 },
    { t: T.keysUp + 60, sound: "whisper" },
    { t: T.pin1, sound: "droplet" },
    { t: T.pin2, sound: "droplet" },
    ...keys([T.finish - 160, T.finish - 80, T.finish], [T.finish + 120, T.finish + 165, T.finish + 210]),
    { t: T.finish + 20, sound: "toggle" },
    { t: T.copied, sound: "chime" },
    { t: T.promptClick, sound: "tick" },
    ...keys([T.paste - 300, T.paste - 150, T.paste], [T.paste + 200, T.paste + 250, T.paste + 270]),
    { t: T.paste + 20, sound: "pulse", volume: 0.8 },
    ...keys([T.submit], [T.submit + 90]),
    { t: T.fix[0], sound: "success" },
  ]
  if (mode === "iphone") cues.push({ t: T.phoneClick, sound: "pulse" })
  return cues.sort((a, b) => a.t - b.t)
}
