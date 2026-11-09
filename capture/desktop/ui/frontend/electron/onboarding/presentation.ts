// Desktop presentation policy. Native adapters supply rectangles in Electron DIP.
export type Rect = readonly [number, number, number, number];
export type Area = {x: number; y: number; width: number; height?: number};

export function coachDimensions(area: Area, phase: string) {
  return {width: Math.min(area.width, phase === 'screen-ink' && area.width >= 720 ? 720 : 380),
    height: Math.min(area.height ?? 440, 440)};
}

export function coachPosition(area: Area, anchor: Rect | undefined | null, board: Rect | undefined | null, phase: string) {
  const {width, height} = coachDimensions(area, phase), gap = 24;
  const bottom = area.y + (area.height ?? 440);
  const clamp = ([x, y]: readonly number[]) => [
    Math.round(Math.max(area.x, Math.min(area.x + area.width - width, x))),
    Math.round(Math.max(area.y, Math.min(bottom - height, y))),
  ] as const;
  const [x, y, w, h] = anchor ?? [area.x + area.width / 2 - 150, area.y + 16, 300, 50];
  const candidates: number[][] = [[x + w / 2 - width / 2, y + h + gap]];
  if (board) {
    const [bx, by, bw, bh] = board;
    // Prefer the board's near side, then every remaining free side. Never use a
    // stale pre-compositor board origin to decide where its drawing surface is.
    candidates.unshift([bx - width - gap, by], [bx + bw + gap, by],
      [bx + bw / 2 - width / 2, by - height - gap],
      [bx + bw / 2 - width / 2, by + bh + gap]);
  }
  const overlap = (p: readonly number[], rect: Rect | undefined | null) => {
    if (!rect) return 0;
    return Math.max(0, Math.min(p[0] + width, rect[0] + rect[2] + 8) - Math.max(p[0], rect[0] - 8)) *
      Math.max(0, Math.min(p[1] + height, rect[1] + rect[3] + 8) - Math.max(p[1], rect[1] - 8));
  };
  const options = candidates.map(clamp);
  // A full-screen board can leave no free side. Minimize covered area rather
  // than blindly falling back to the first clamped (overlapping) candidate.
  const position = options.find(p => !overlap(p, board) && !overlap(p, anchor)) ??
    options.reduce((best, p) => overlap(p, board) + overlap(p, anchor) < overlap(best, board) + overlap(best, anchor) ? p : best);
  return {x: position[0], y: position[1], arrow: Math.max(45, Math.min(width - 45, x + w / 2 - position[0])),
    pointsUp: !board && !!anchor && ['screen', 'whiteboard', 'stop', 'copied', 'locate'].includes(phase)};
}

export function practiceExit(phase: string) {
  const recording = ['whiteboard', 'board-ink', 'screen-ink', 'note', 'stop'].includes(phase);
  return {label: 'Close guide', detail: recording ? 'Recording continues.' : phase === 'screen' ? 'Capture controls stay available.' :
    'Capture and setup progress kept.'};
}
