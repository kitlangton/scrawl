# Moving recording controls — issue #116

## Task and current behavior

During a screen capture of Apparent itself, Simon needs to show an agent both the recording controls and the screen beneath their previous position. The capsule must move without stopping the recording or obscuring Finish. The previous exclusion request (#34) is closed and is not this feature.

Before this feature, the recording capsule is centred near the top of the selected display. Linux uses a GPUI capsule in a 640 × 84 override-redirect XWayland window; placement uses `x11::place`, which remaps on Hyprland. The macOS default is a 640 × 84 AppKit/Metal panel with pointer-polled hit testing, while the optional GPUI renderer has a native window-move region in its leading status cluster. The Linux status cluster has no drag mechanic. Mac ScreenCaptureKit excludes Apparent's own floating windows from both the movie and screenshot samples; Linux captures them in the composited movie but masks the pill only in small candidate images used to choose moments. Full-quality keyframes decode from the movie. These are source observations, not GUI proof.

## Interaction choice

Use the *leading status/clock cluster* as the handle during recording, not the whole capsule: dragging Finish, Cancel, mode chips or Whiteboard would compete with their clicks. Keep the grab offset, clamp the **whole panel window** to its current display, and leave the dropped position alone for that capture. No edge snap or persistent preference: it would make a capsule jump just when the user is pointing at uncovered content. A new capture restores the ordinary centred placement. Do not follow pointer/display crossings after a manual move; crossing to another display is a distinct move of the whole window, not a retargeting of the capture stream. Clicks on actionable controls keep their existing behavior. The existing recording-timer accessibility label remains; a dedicated movable hint and grab cursor are deferred rather than making the small handle wider or changing actionable controls. The drag must not steal Shift ink or ordinary clicks into the content below the moved capsule.

The Mac native renderer and GPUI renderer use the same recording-only drag policy and display-bounds calculation; AppKit/X11 perform their own pointer and window mechanics. Mac display capture must include the capsule in **both** stream and screenshot filters while keeping the separate system-permission-prompt privacy handling. Linux's candidate-only activity mask may stay, since it does not alter the delivered movie or full-quality decoded keyframes.

## Acceptance matrix

| Behavior | Deterministic regression | Linux GUI and bundle | Mac GUI and bundle |
| --- | --- | --- | --- |
| Grab lead; Finish/Cancel/Whiteboard and mode chips still click | input hit and drag tests | click controls during capture | native and optional GPUI controls |
| Move while recording, remain in bounds, no snap until end | shared geometry and session reset tests | compositor geometry, off-edge drag | native panel frame, off-edge drag |
| New capture starts centred; target/display unchanged | coordinator start/reset tests | two captures, same selected stream | two captures |
| Own controls and revealed actual screen remain visible | Mac filter regression | decoded movie plus full-quality keyframes and contact sheet | decoded movie plus full-quality keyframes and contact sheet |
| Interactions survive | unchanged event/ink path | click, Shift ink, whiteboard, Finish | click, Shift ink, whiteboard, Finish |

A VM missing the H.264 decoder cannot establish full-quality keyframe acceptance; a Mac build without a real GUI capture cannot establish native-panel acceptance. Do not describe the PR as mergeable if either platform lacks the stated proof. No merge, deployment or release is part of this issue.

This behavior is on by default. Opt out with `"experiments": {"movableRecordingControls": false}` in `~/.config/apparent/config.json`. Mac capture filtering and the drag handle both follow that switch on the next capture; Linux's candidate-only activity mask is unchanged.
