// A gallery of auto-crop results, for judging them by eye: per bundle, each full keyframe with
// its crop (red) and the hand-labelled ideal (green, when the bundle has a truth.json), the
// cropped keyframe as written, and the cropped contact sheet beside the full one.
//
//   bun public/site/scripts/autocrop-gallery.mjs <out-dir> <bundle-or-dir> ... [--scores scores.json]
//
// Writes <out-dir>/<bundle>.jpg per bundle, <out-dir>/overview.jpg and <out-dir>/index.html.

import { mkdirSync, writeFileSync, readFileSync, existsSync, readdirSync } from 'node:fs';
import { join, resolve, basename } from 'node:path';
import { pathToFileURL } from 'node:url';

const args = process.argv.slice(2);
const scoresAt = args.indexOf('--scores');
const scores = scoresAt >= 0 ? JSON.parse(readFileSync(args.splice(scoresAt, 2)[1], 'utf8')) : [];
const [outArg, ...inputs] = args;
const OUT = resolve(outArg);
mkdirSync(OUT, { recursive: true });

const bundles = inputs.flatMap((p) => {
  p = resolve(p);
  if (existsSync(join(p, 'apparent.json'))) return [p];
  return readdirSync(p).map((n) => join(p, n)).filter((d) => existsSync(join(d, 'apparent.json'))).sort();
});

async function loadPlaywright() {
  try { return await import('playwright'); } catch {}
  const dir = new URL('../../node_modules/playwright/index.mjs', import.meta.url).pathname;
  if (existsSync(dir)) return await import(dir);
  throw new Error('Playwright not found');
}

const url = (p) => pathToFileURL(p).href;
const esc = (s) => String(s ?? '').replace(/&/g, '&amp;').replace(/</g, '&lt;');
const CSS = `
body { margin: 0; background: #101114; color: #e8e8ec; font: 13px/1.45 -apple-system, sans-serif; }
.sc { padding: 22px 26px; width: 1640px; }
h2 { margin: 0 0 2px; font-size: 20px; } .desc { color: #9aa0ab; margin-bottom: 4px; } .score { color: #ffcf6b; margin-bottom: 14px; }
.kf { display: grid; grid-template-columns: 620px 620px 1fr; gap: 16px; margin-bottom: 18px; align-items: start; }
.full { position: relative; width: 620px; } .full img, .crop img { width: 100%; display: block; border-radius: 4px; }
.crop { max-height: 460px; display: flex; } .crop img { max-height: 460px; width: auto; max-width: 620px; object-fit: contain; }
.box { position: absolute; box-sizing: border-box; }
.box.crop-r { border: 3px solid #ff4d4f; }
.box.ideal { border: 2px dashed #3ddc84; }
.meta b { color: #fff; } .meta .why { color: #ff8a8a; margin-top: 6px; } .meta .said { color: #c9ccd3; margin-top: 6px; font-style: italic; }
.meta .miss { color: #ff4d4f; font-weight: 700; }
.sheets { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; } .sheets img { width: 100%; border-radius: 4px; } .sheets div { color: #9aa0ab; margin-bottom: 4px; }
`;

function section(dir) {
  const b = JSON.parse(readFileSync(join(dir, 'apparent.json'), 'utf8'));
  const truth = existsSync(join(dir, 'truth.json')) ? JSON.parse(readFileSync(join(dir, 'truth.json'), 'utf8')) : null;
  const name = truth?.scenario || basename(dir).replace(/\.apparent$/, '');
  const sc = scores.find((s) => s.name === name);
  const [W, H] = [b.target.frame.width, b.target.frame.height];
  const crops = b.autocrop?.crops || [];
  const fullDir = b.autocrop?.fullFrames || 'frames';
  const pct = (v, d) => `${(v / d) * 100}%`;
  const box = (cls, [x, y, w, h]) => `<div class="box ${cls}" style="left:${pct(x, W)};top:${pct(y, H)};width:${pct(w, W)};height:${pct(h, H)}"></div>`;
  const rows = b.keyframes.map((kf, i) => {
    const file = basename(kf.file);
    const c = crops[i];
    const span = truth?.spans.find((s) => s.t0 <= kf.t && kf.t < s.t1) || truth?.spans.at(-1);
    const ks = sc?.keyframes[i];
    return `<div class="kf"><div class="full"><img src="${url(join(dir, fullDir, file))}">${span ? box('ideal', span.ideal) : ''}${c && c.kind !== 'full' ? box('crop-r', [c.x, c.y, c.width, c.height]) : ''}</div>
      <div class="crop"><img src="${url(join(dir, 'frames', file))}"></div>
      <div class="meta"><b>#${kf.index} · ${(kf.t / 1000).toFixed(1)}s · ${esc(kf.reason)}</b>${ks ? ` · IoU ${ks.iou.toFixed(2)} · score ${ks.score.toFixed(2)}${ks.covered ? '' : ' · <span class="miss">misses the activity</span>'}` : ''}
        <div class="why">${c ? `${esc(c.kind)}: ${esc(c.reason)} · ${Math.round(c.width)}×${Math.round(c.height)} pt` : 'not cropped'}</div>
        ${span ? `<div>ideal: ${esc(span.subject)} · ${span.ideal[2]}×${span.ideal[3]} pt</div>` : ''}
        <div class="said">${esc(kf.said || kf.event)}</div></div></div>`;
  }).join('');
  const sheets = existsSync(join(dir, 'contact-sheet-full.png'))
    ? `<div class="sheets"><div><div>contact-sheet.png (cropped)</div><img src="${url(join(dir, 'contact-sheet.png'))}"></div><div><div>contact-sheet-full.png</div><img src="${url(join(dir, 'contact-sheet-full.png'))}"></div></div>`
    : `<div class="sheets"><div><div>contact-sheet.png (not cropped)</div><img src="${url(join(dir, 'contact-sheet.png'))}"></div></div>`;
  return { name, html: `<div class="sc" id="${name}"><h2>${esc(name)}</h2><div class="desc">${esc(truth?.description || b.note || '')}${b.autocrop ? ` · cropped to ${esc(b.autocrop.label)}` : ''}</div>
    ${sc ? `<div class="score">score ${sc.score.toFixed(3)} · coverage ${Math.round(sc.covered * 100)}% · stability ${sc.stability.toFixed(2)}</div>` : ''}${rows}${sheets}</div>` };
}

const sections = bundles.map(section);
const page = (body) => `<!doctype html><html><head><meta charset="utf-8"><style>${CSS}</style></head><body>${body}</body></html>`;
writeFileSync(join(OUT, 'index.html'), page(sections.map((s) => s.html).join('')));

// Overview: one cropped keyframe per scenario next to its full frame with the box.
const tiles = bundles.map((dir) => {
  const b = JSON.parse(readFileSync(join(dir, 'apparent.json'), 'utf8'));
  const truth = existsSync(join(dir, 'truth.json')) ? JSON.parse(readFileSync(join(dir, 'truth.json'), 'utf8')) : null;
  const i = Math.min(1, b.keyframes.length - 1), kf = b.keyframes[i], c = b.autocrop?.crops[i];
  const [W, H] = [b.target.frame.width, b.target.frame.height];
  const fullDir = b.autocrop?.fullFrames || 'frames', file = basename(kf.file);
  const pct = (v, d) => `${(v / d) * 100}%`;
  return `<div class="tile"><div class="full"><img src="${url(join(dir, fullDir, file))}">${c && c.kind !== 'full' ? `<div class="box crop-r" style="left:${pct(c.x, W)};top:${pct(c.y, H)};width:${pct(c.width, W)};height:${pct(c.height, H)}"></div>` : ''}</div>
    <div class="cropped"><img src="${url(join(dir, 'frames', file))}"></div><div class="cap"><b>${esc(truth?.scenario || basename(dir))}</b> #${kf.index} · ${esc(c ? c.reason : 'not cropped')}</div></div>`;
}).join('');
const OV = `.grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 18px; padding: 22px; width: 2100px; }
.tile { background: #191a1f; border-radius: 8px; padding: 10px; } .tile .full { position: relative; width: 100%; } .tile .full img { width: 100%; display: block; }
.tile .cropped { margin-top: 8px; height: 300px; display: flex; justify-content: center; background: #0c0c0e; } .tile .cropped img { height: 100%; max-width: 100%; object-fit: contain; }
.cap { margin-top: 6px; color: #c9ccd3; } .cap b { color: #fff; }
h1 { padding: 22px 22px 0; margin: 0; font-size: 22px; } .sub { padding: 0 22px; color: #9aa0ab; }`;
writeFileSync(join(OUT, 'overview.html'), `<!doctype html><html><head><meta charset="utf-8"><style>${CSS}${OV}</style></head><body><h1>Apparent auto-crop · simulated Screen captures</h1><div class="sub">Each tile: the full keyframe with the crop (red), and the cropped keyframe Apparent writes to frames/.</div><div class="grid">${tiles}</div></body></html>`);

const { chromium } = await loadPlaywright();
const browser = await chromium.launch();
const pg = await browser.newPage({ viewport: { width: 1700, height: 1000 } });
await pg.goto(url(join(OUT, 'index.html')));
await pg.waitForLoadState('load');
for (const s of sections) {
  await pg.locator(`#${s.name}`).screenshot({ path: join(OUT, `${s.name}.jpg`), type: 'jpeg', quality: 88 });
  console.log(join(OUT, `${s.name}.jpg`));
}
await pg.setViewportSize({ width: 2150, height: 1000 });
await pg.goto(url(join(OUT, 'overview.html')));
await pg.waitForLoadState('load');
await pg.screenshot({ path: join(OUT, 'overview.jpg'), type: 'jpeg', quality: 88, fullPage: true });
console.log(join(OUT, 'overview.jpg'));
await browser.close();
