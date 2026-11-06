// Observed by visual inspection of the approved rendered demo; no embedded cursor telemetry
// was found. Coordinates are normalized to the full 1916×1164 frame. Source SHA-256:
// 89c4621457e573d58a00f99714682eb4bf9351b59de95cabebb944007654d7a7
export type DemoCursorPoint = { t: number; x: number; y: number }
export type DemoCaption = { start: number; end: number; text: string }

// Authored from sampled visible arrow/crosshair and interaction positions, not telemetry.
// Initialize on the first stable arrow; hold hidden spans. The result returns to the
// browser reload interaction rather than leaving the viewer on the terminal.
export const DEMO_CURSOR: readonly DemoCursorPoint[] = [
  { t: 0, x: .353, y: .634 },
  { t: .5, x: .353, y: .634 },
  { t: 3.5, x: .353, y: .634 },
  { t: 4.5, x: .372, y: .300 },
  { t: 5.5, x: .304, y: .325 },
  { t: 7.5, x: .304, y: .325 },
  { t: 8.5, x: .355, y: .730 },
  { t: 10.5, x: .368, y: .407 },
  { t: 14.5, x: .368, y: .407 },
  { t: 16.5, x: .557, y: .022 },
  { t: 17.5, x: .553, y: .164 },
  { t: 18.5, x: .647, y: .750 },
  { t: 19.5, x: .709, y: .626 },
  { t: 20.5, x: .641, y: .751 },
  { t: 21.5, x: .641, y: .751 },
  { t: 22.5, x: .612, y: .590 },
  { t: 23.5, x: .608, y: .543 },
  { t: 25.5, x: .708, y: .585 },
  { t: 26.5, x: .660, y: .693 },
  { t: 28.0, x: .660, y: .693 },
  { t: 29.5, x: .625, y: .022 },
  { t: 31.5, x: .625, y: .420 },
  { t: 32.5, x: .625, y: .420 },
  { t: 36.5, x: .625, y: .420 },
  { t: 44.5, x: .625, y: .420 },
  { t: 46.5, x: .625, y: .420 },
  { t: 47.5, x: .082, y: .110 },
  { t: 49.0, x: .082, y: .110 },
  { t: 50.0, x: .353, y: .634 },
  { t: 59.82, x: .353, y: .634 },
]

// Cue bounds are approximate 1 fps visual bounds; exact words are transcribed from burned
// captions. No words are supplied for the visibly missing/unclear spans.
export const DEMO_CAPTIONS: readonly DemoCaption[] = [
  { start: 0, end: 4.8, text: "I want to change the ghost in my little Halloween invitation here," },
  { start: 5.0, end: 6.6, text: "so I'm going to start Apparent," },
  { start: 6.6, end: 8.3, text: "select the screen, and say," },
  { start: 8.3, end: 10.8, text: "hey, can you change this ghost here," },
  { start: 10.8, end: 12.8, text: "and make the eyes bigger." },
  { start: 13.0, end: 15.5, text: "But now maybe I have a specific idea in mind already." },
  { start: 15.5, end: 17.0, text: "so I can also just open up the whiteboard" },
  { start: 17.0, end: 19.2, text: "and draw the ghost exactly the way I want it to look," },
  { start: 19.2, end: 21.8, text: "something like this, squiggly lines," },
  { start: 21.8, end: 23.8, text: "and then the eyes look real big," },
  { start: 23.8, end: 25.6, text: "but actually maybe circles like that," },
  { start: 25.6, end: 27.7, text: "and a little smile, keep that." },
  { start: 27.7, end: 30.5, text: "Then I can stop the Apparent recording," },
  { start: 30.5, end: 32.0, text: "it gets processed," },
  { start: 32.0, end: 35.7, text: "and then I can paste it into my agent chat like this." },
  { start: 35.7, end: 37.1, text: "And you can see here," },
  { start: 37.1, end: 39.5, text: "it sends a picture of the recording," },
  { start: 39.5, end: 41.0, text: "some explanation how to use it," },
  { start: 41.0, end: 44.8, text: "and then also step-by-step the transcript, all in one package." },
  { start: 46.0, end: 47.8, text: "Now the agent responded," },
  { start: 47.8, end: 50.0, text: "and now you can reload, and there you go." },
  { start: 50.2, end: 52.0, text: "It looks exactly like I drew it," },
  { start: 52.0, end: 54.0, text: "maybe a bit janky, but I really like it," },
  { start: 54.0, end: 55.2, text: "I think it has a lot of personality." },
  { start: 56.0, end: 57.3, text: "So yeah, I hope you like this demo," },
  { start: 57.3, end: 58.1, text: "and give it a try," },
  { start: 58.1, end: 59.3, text: "and give me some feedback. Thanks." },
]
