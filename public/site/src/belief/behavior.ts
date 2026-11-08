// Same approved poses and direct scroll mapping in the source build and live release.
export function mountBelief(root: HTMLElement): () => void {
function required<T extends Element>(selector: string): T {
  const node = root.querySelector<T>(selector);
  if (!node) throw new Error(`Missing belief element: ${selector}`);
  return node;
}
const study = required<HTMLElement>('#study');
const button = required<HTMLButtonElement>('#make-room');
const status = required<HTMLElement>('#status');
const scene = required<HTMLElement>('.scene');
const left = required<HTMLElement>('.pair-left');
const right = required<HTMLElement>('.pair-right');
const newcomer = required<HTMLElement>('.newcomer');
const hug = required<HTMLElement>('.hug');
const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)');
let animations: Animation[] = [];
let timers: number[] = [];
let run = 0;
let active = true;

function clearAnimations() {
  timers.forEach(clearTimeout);
  timers = [];
  animations.forEach(animation => animation.cancel());
  animations = [];
}
function finish() {
  run += 1;
  study.dataset.state = 'hugged';
  clearAnimations();
  button.removeAttribute('aria-disabled');
  button.textContent = 'Again';
  status.textContent = 'Right where you belong.';
  scene.setAttribute('aria-label', 'Three olive frogs sit close together, arms wrapped around each other in a group hug.');
}
function reset() {
  run += 1;
  clearAnimations();
  study.dataset.state = 'idle';
  button.removeAttribute('aria-disabled');
  newcomer.dataset.pose = 'rest';
  button.textContent = 'Me too';
  status.textContent = 'A place for you, right in the middle.';
  scene.setAttribute('aria-label', 'Two frogs wait with a gap between them. A third frog sits off to the left.');
}
function animate(element: HTMLElement, frames: Keyframe[], options: KeyframeAnimationOptions) {
  const animation = element.animate(frames, { fill: 'both', ...options });
  animations.push(animation);
  return animation;
}
async function makeRoom() {
  if (study.dataset.state === 'moving') return;
  if (study.dataset.state === 'hugged') { reset(); return; }
  if (reduceMotion.matches) { finish(); return; }
  const currentRun = ++run;
  study.dataset.state = 'moving';
  button.setAttribute('aria-disabled', 'true');
  status.textContent = 'Jumping in…';
  scene.setAttribute('aria-label', 'Their friend makes one leap from the left into the gap between the waiting frogs.');
  // Same 970ms arc, anticipation, flight poses and landing squash as
  // public/site/src/FrogPond.tsx. Only distance and arc height fit this smaller stage.
  // Percentage x keeps the landing in the central gap if the viewport resizes.
  const distance = 69.733 / 26.333 * 100;
  // The heading wraps independently of scene width. Keep the apex below it,
  // including on wide screens where the proportional arc can reach the text.
  const clearance = newcomer.getBoundingClientRect().top - belief.getBoundingClientRect().bottom;
  const arc = Math.max(0, Math.min(scene.clientWidth * .12, clearance - 8));
  const transform = (progress: number, y: number, squash = '') => `translate(${distance * progress}%, ${y}px) ${squash}`;
  animate(newcomer, [
    { transform: transform(0, 0), offset: 0 },
    { transform: transform(0, 0, 'scaleY(.83)'), offset: .16 },
    { transform: transform(.23, -arc * .72), offset: .35 },
    { transform: transform(.52, -arc), offset: .54 },
    { transform: transform(.82, -arc * .48), offset: .79 },
    { transform: transform(1, 0, 'scaleY(.84)'), offset: .93 },
    { transform: transform(1, 0), offset: 1 },
  ], { duration: 970, easing: 'linear' });
  timers.push(window.setTimeout(() => { newcomer.dataset.pose = 'air'; }, 160));
  timers.push(window.setTimeout(() => { newcomer.dataset.pose = 'rest'; }, 840));
  timers.push(window.setTimeout(() => { status.textContent = 'And… a hug.'; }, 970));
  for (const frog of [left, right, newcomer]) {
    animate(frog, [{ opacity: 1 }, { opacity: 0 }], { duration: 140, delay: 1060 });
  }
  const settle = animate(hug, [
    { opacity: 0, transform: 'scale(1)' },
    { opacity: 1, transform: 'scale(1.025, .975)', offset: .6 },
    { opacity: 1, transform: 'scale(1)' },
  ], { duration: 220, delay: 1060 });
  try {
    await settle.finished;
    if (active && currentRun === run) finish();
  } catch { /* Cancelled when reduced-motion preference changes. */ }
}
// A disconnected element may be reattached with its interrupted attributes intact.
reset();
button.addEventListener('click', makeRoom);
function onMotionChange(event: MediaQueryListEvent) {
  if (event.matches && study.dataset.state === 'moving') finish();
  drawScrollMarks();
}

// Direct, reversible scroll mapping. No timers, easing, transition or lerp.
const belief = required<HTMLElement>('#belief');
const circle = required<SVGPathElement>('.work-circle');
const highlight = required<SVGPathElement>('.enjoy-highlight');
const clamp = (value: number) => Math.max(0, Math.min(1, value));
function drawScrollMarks() {
  if (!active) return;
  const progress = clamp((innerHeight * .70 - belief.getBoundingClientRect().top) / (innerHeight * .58));
  const circleProgress = reduceMotion.matches ? 1 : clamp(progress / .48);
  const highlightProgress = reduceMotion.matches ? 1 : clamp((progress - .55) / .45);
  circle.style.strokeDashoffset = String(1 - circleProgress);
  circle.style.opacity = circleProgress > 0 ? '1' : '0';
  highlight.style.strokeDashoffset = String(1 - highlightProgress);
  study.dataset.circle = String(circleProgress);
  study.dataset.highlight = String(highlightProgress);
}
addEventListener('scroll', drawScrollMarks, { passive: true });
function onResize() {
  if (study.dataset.state === 'moving') finish();
  drawScrollMarks();
}
addEventListener('resize', onResize);
reduceMotion.addEventListener('change', onMotionChange);
document.fonts.ready.then(drawScrollMarks);
drawScrollMarks();

return () => {
  active = false;
  run += 1;
  clearAnimations();
  button.removeEventListener('click', makeRoom);
  removeEventListener('scroll', drawScrollMarks);
  removeEventListener('resize', onResize);
  reduceMotion.removeEventListener('change', onMotionChange);
};
}

// Custom-element lifecycle keeps React hydration and detached animations separate.
if (typeof customElements !== 'undefined' && !customElements.get('apparent-belief')) {
  class ApparentBelief extends HTMLElement {
    private cleanup?: () => void;
    connectedCallback() { this.cleanup?.(); this.cleanup = mountBelief(this); }
    disconnectedCallback() { this.cleanup?.(); this.cleanup = undefined; }
  }
  customElements.define('apparent-belief', ApparentBelief);
}
