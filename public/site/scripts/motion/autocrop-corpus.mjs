// Simulated Screen captures with ground truth, for tuning and testing auto-crop.
//
// Each scenario is a short script of what someone did on a 1728×1117 pt MacBook Pro screen:
// windows (a terminal running OpenCode, a browser app, an editor, chat, Finder, Notes), the menu
// bar and Dock, narration with word timings, Shift-drawn ink, clicks, drags, scrolls, human-ish
// cursor motion, windows moving and focusing. The script renders every distinct screen state
// with Playwright at Apparent's Screen sampling size (2560 px wide) and writes `scenario.json`
// (what Apparent's input monitors and window log would have recorded) plus `truth.json` (the crop a
// person would want for each stretch of the capture, in points). `apparent fixture` then builds a
// real bundle from it: keyframe selection, ink and click ripples, and auto-crop.
//
//   bun public/site/scripts/autocrop-corpus.mjs <out-dir> [scenario-name ...]
//   for s in <out-dir>/*/scenario.json; do apparent fixture $s --out ${s%/scenario.json}.apparent; done
//   apparent autocrop-eval <out-dir>
//
// Playwright comes from any node_modules that has it (this repo's public/site/ does).

import { mkdirSync, writeFileSync, existsSync } from 'node:fs';
import { join, resolve } from 'node:path';

const OUT = resolve(process.argv[2] || 'autocrop-corpus');
const ONLY = new Set(process.argv.slice(3));
const SW = 1728, SH = 1117, MENU = 33, FRAME_W = 2560;
const DOCK_H = 74;

// ---------------------------------------------------------------------------------------------
// Deterministic randomness
// ---------------------------------------------------------------------------------------------
let seed = 1;
const rand = () => { seed = (seed * 1664525 + 1013904223) >>> 0; return seed / 2 ** 32; };
const jitter = (a) => (rand() * 2 - 1) * a;

// ---------------------------------------------------------------------------------------------
// Window content (HTML). Elements worth pointing at carry data-t="name".
// ---------------------------------------------------------------------------------------------
const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;');
const lorem = 'the quick settings panel loads the account details from the api and caches them for the session so that every page can render without waiting on the network again unless the token expires'.split(' ');
const words = (n, k = 0) => Array.from({ length: n }, (_, i) => lorem[(i * 7 + k * 3) % lorem.length]).join(' ');

function terminal(o) {
  const lines = [
    `<div class="tui-head"><b>opencode</b><span>~/code/acme</span><span>claude-opus-4</span><span class="dim">48.2k tokens · $0.41</span></div>`,
    `<div class="msg user">Run the settings tests and fix whatever is failing in the account card.</div>`,
    `<div class="msg">I'll run the test suite for the settings package first.</div>`,
    `<div class="tool" data-t="tool"><div class="tool-h">▸ Bash <span class="dim">bun test src/settings</span></div>
      <pre>bun test v1.2.4
 src/settings/AccountCard.test.tsx:
 <span class="ok">✓</span> renders the account name <span class="dim">[3.1ms]</span>
 <span class="ok">✓</span> renders the plan badge <span class="dim">[1.2ms]</span>
 <span class="bad" data-t="error">✗ header keeps its top inset</span> <span class="dim">[0.9ms]</span>
   <span data-t="expect">Expected: 0px   Received: -26px</span>
 <span class="ok">✓</span> saves on submit <span class="dim">[4.4ms]</span>

 <span class="bad">1 fail</span> · <span class="ok">3 pass</span> · 4 expect() calls</pre></div>`,
    `<div class="msg">The header is pulled up by a negative margin in <code>AccountCard.tsx</code>. Here's the fix:</div>`,
    `<div class="tool" data-t="diff"><div class="tool-h">▸ Edit <span class="dim">src/settings/AccountCard.tsx</span></div>
      <pre><span class="dim">  41</span>  &lt;section className="card"&gt;
<span class="del"><span class="dim">  42</span> -  &lt;h2 style={{ marginTop: -26 }}&gt;Account&lt;/h2&gt;</span>
<span class="add"><span class="dim">  42</span> +  &lt;h2&gt;Account&lt;/h2&gt;</span>
<span class="dim">  43</span>    &lt;Row label="Name" value={user.name} /&gt;</pre></div>`,
    `<div class="msg">Re-running the tests now.</div>`,
    ...(o.more ? [`<div class="tool"><div class="tool-h">▸ Bash <span class="dim">bun test src/settings</span></div><pre> <span class="ok">4 pass</span> · 0 fail</pre></div>`, `<div class="msg">All four settings tests pass. ${words(24, 2)}.</div>`] : []),
  ];
  return `<div class="tui">${lines.join('')}<div class="tui-input" data-t="prompt"><span data-t="prompt-text"><span class="caret">❯</span> ${esc(o.prompt || 'also check the slider')}</span><span class="cursor-blk"></span></div>
    <div class="tui-foot"><span>build</span><span class="dim">tab agents · ctrl+p commands</span></div></div>`;
}

function browserPage(o) {
  const toast = o.toast ? `<div class="toast" data-t="toast">Couldn't save changes · <u>Retry</u></div>` : '';
  const dropdown = o.notifications ? `<div class="notif" data-t="notifs"><b>Notifications</b>
      <div class="n-item"><span data-t="n-0">Deploy <b>web-412</b> failed on <i>main</i></span></div><div class="n-item">Kit invited you to <b>acme/web</b></div><div class="n-item">Usage is at 92% of your plan</div></div>` : '';
  const bars = [42, 58, 51, 66, 80, 74, 92, 61, 70, 88, 95, 77].map((v, i) => `<i style="height:${v}%" ${i === 10 ? 'data-t="spike"' : ''}></i>`).join('');
  const rows = ['web-409 · main · 2m ago · Ready', 'web-410 · fix/slider · 9m ago · Ready', 'web-411 · main · 21m ago · Ready', 'web-412 · main · 1h ago · Failed', 'web-413 · docs · 2h ago · Ready', 'web-414 · main · 3h ago · Ready']
    .map((r, i) => `<div class="tr${i === 3 ? ' fail' : ''}" data-t="row-${i}">${r.split(' · ').map((c) => `<span>${c}</span>`).join('')}</div>`).join('');
  return `<div class="app">
    <div class="nav"><div class="logo">▲ acme</div>${['Overview', 'Deployments', 'Analytics', 'Logs', 'Storage', 'Settings'].map((n) => `<div class="${n === (o.tab || 'Settings') ? 'on' : ''}" data-t="nav-${n}">${n}</div>`).join('')}</div>
    <div class="main">
      <div class="topbar"><span class="crumb">acme / web / ${o.tab || 'Settings'}</span><span class="search">Search…  ⌘K</span><span class="bell" data-t="bell">🔔<b class="badge" data-t="badge">3</b></span><span class="avatar">KL</span>${dropdown}</div>
      <h1>${o.tab || 'Settings'}</h1>
      <div class="grid">
        <div class="card" data-t="card-account"><h3 data-t="card-head">Account</h3><div class="row"><span>Name</span><span>Kit Langton</span></div><div class="row"><span>Email</span><span>kit@example.com</span></div><div class="row"><span>Plan</span><span class="pill">Pro</span></div></div>
        <div class="card" data-t="chart"><h3>Requests · 12h</h3><div class="bars">${bars}</div><div class="dim small"><span data-t="peak">Peak 18.2k/min at 09:00</span></div></div>
        <div class="card wide" data-t="table"><h3>Recent deployments</h3>${rows}</div>
        <div class="card"><h3>Notification volume</h3><div class="slider" data-t="slider"><i style="left:${o.slider ?? 30}%"></i></div><div class="dim small">Drag to adjust</div></div>
        <div class="card"><h3>Danger zone</h3><div class="dim small">${words(18, 4)}</div></div>
      </div>
      <div class="actions"><span class="btn ghost">Cancel</span><span class="btn" data-t="save">Save changes</span></div>${toast}
    </div></div>`;
}

function docsPage(o) {
  const secs = ['Installation', 'Configuration', 'Authentication', 'Rate limits', 'Webhooks', 'Pagination', 'Errors', 'Changelog'];
  const body = secs.map((s, i) => `<h2 data-t="h-${i}">${i + 1}. ${s}</h2>
      <p data-t="para-${i}">${words(60, i)}. ${i === 3 ? '<b data-t="rule">Requests over 600 per minute return <code data-t="code-429">429 Too Many Requests</code> with a <code>Retry-After</code> header.</b> ' : ''}${words(40, i + 1)}.</p>
      <pre class="code">curl https://api.acme.dev/v1/${s.toLowerCase().replace(/ /g, '-')} \\\n  -H "Authorization: Bearer $TOKEN"</pre>
      <p>${words(50, i + 2)}. <a data-t="link-${i}">Read more about ${s.toLowerCase()} →</a></p>`).join('');
  return `<div class="docs"><div class="toc">${secs.map((s, i) => `<div data-t="toc-${i}">${s}</div>`).join('')}</div><div class="article" data-scroll-to="${o.scrollTo || ''}" data-scroll-frac="${o.scrollFrac ?? 0}"><h1>API reference</h1>${body}</div></div>`;
}

function browser(o) {
  return `<div class="bbar"><span class="lights"><i></i><i></i><i></i></span><span class="tabs"><span class="tab on">${esc(o.title)}</span><span class="tab">GitHub</span><span class="tab">Linear</span></span></div>
    <div class="burl"><span>‹ ›</span><span class="url" data-t="url">${o.url || 'acme.dev/web/settings'}</span><span class="dim">⋯</span></div>
    <div class="bcontent" data-t="view">${o.page === 'docs' ? docsPage(o) : browserPage(o)}</div>`;
}

function editor(o) {
  const code = [
    ['kw', 'import'], ['', ' { useState } '], ['kw', 'from'], ['str', " 'react'"], null,
    ['kw', 'export function'], ['fn', ' AccountCard'], ['', '({ user }: Props) {'], null,
    ['', '  const [saving, setSaving] = '], ['fn', 'useState'], ['', '(false)'], null,
    ['', '  return ('], null, ['', '    <section className="card">'], null,
    ['', '      <h2 style={{ marginTop: '], ['num', '-26'], ['', ' }}>Account</h2>'], null,
    ['', '      <Row label="Name" value={user.name} />'], null, ['', '      <Row label="Email" value={user.email} />'], null,
    ['', '      <Plan plan={user.plan} />'], null, ['', '    </section>'], null, ['', '  )'], null, ['', '}'], null, null,
    ['kw', 'function'], ['fn', ' Row'], ['', '({ label, value }) {'], null, ['', '  return <div className="row"><span>{label}</span>{value}</div>'], null, ['', '}'], null,
  ];
  const lines = []; let cur = '';
  for (const c of code) { if (c === null) { lines.push(cur); cur = ''; } else cur += `<span class="${c[0]}">${esc(c[1])}</span>`; }
  const body = lines.map((l, i) => `<div class="ln${i === 7 ? ' hl' : ''}" data-t="line-${i}"><span class="no">${i + 1}</span><span data-t="code-${i}">${l}</span>${i === 7 ? '<span class="diag" data-t="diag">⚠ negative margin pulls the header out of the card</span>' : ''}</div>`).join('');
  const files = ['src', '  settings', '    AccountCard.tsx', '    Slider.tsx', '    index.ts', '  api', '    client.ts', 'package.json', 'README.md'];
  return `<div class="ed"><div class="tree" data-t="tree">${files.map((f) => `<div class="${f.includes('AccountCard') ? 'on' : ''}">${f.replace(/ /g, '&nbsp;')}</div>`).join('')}</div>
    <div class="code"><div class="etabs"><span class="on" data-t="tab">AccountCard.tsx</span><span>Slider.tsx</span></div>${body}
    <div class="status" data-t="status"><span data-t="status-text">⚠ 1 warning · TypeScript · Ln 8, Col 31</span></div></div></div>`;
}

function chat(o) {
  const msgs = [
    ['Mara', 'deploy for web-412 failed again, anyone looking?'], ['Kit', 'yep, it is the settings header test, fixing now'],
    ['Jon', 'is this the same one from yesterday? the account card thing'], ['Mara', words(14, 3)],
    ['Kit', 'pushed a fix, waiting on CI'], ['Jon', 'ty! also the slider in notifications does nothing for me'],
  ];
  return `<div class="slack"><div class="ws"><b>Anomaly</b>${['# general', '# core', '# deploys', '# design', '# random'].map((c) => `<div class="${c === '# deploys' ? 'on' : ''}">${c}</div>`).join('')}<div class="dim">Direct messages</div><div>Mara</div><div>Jon</div></div>
    <div class="chan"><div class="chead"># deploys <span class="dim">· 14 members</span></div>${msgs.map(([who, m], i) => `<div class="cm" data-t="msg-${i}"><span class="av">${who[0]}</span><div><b>${who}</b> <span class="dim">10:${12 + i}</span><div>${esc(m)}</div></div></div>`).join('')}
    <div class="composer" data-t="composer">Message #deploys</div></div></div>`;
}

function finder(o) {
  const files = ['Screenshots', 'invoice-0912.pdf', 'design-v3.fig', 'notes.md', 'recording.mov', 'budget.xlsx', 'logo.svg', 'archive.zip'];
  const items = o.list
    ? `<div class="flist">${files.map((f, i) => `<div><span data-t="file-${i}">${f}</span><span class="dim">Oct ${3 + i}</span><span class="dim">${(i + 1) * 213} KB</span></div>`).join('')}</div>`
    : `<div class="fgrid">${files.map((f, i) => `<div data-t="file-${i}"><i></i>${f}</div>`).join('')}</div>`;
  return `<div class="fbar"><span class="lights"><i></i><i></i><i></i></span><span>‹ ›</span><b>Downloads</b><span class="views" data-t="views">⊞ ☰ ⫼</span></div>
    <div class="finder"><div class="fside"><div class="dim">Favourites</div><div>AirDrop</div><div>Recents</div><div>Applications</div><div class="on">Downloads</div><div>Documents</div></div>${items}</div>`;
}

function notes(o) {
  return `<div class="tbar"><span class="lights"><i></i><i></i><i></i></span><span class="ttl">Notes — Bugs</span></div><div class="notes"><h2>Bugs this week</h2><ul>
    <li><span data-t="note-0">Account header clipped by 26px</span></li><li><span data-t="note-1">Slider ignores drags</span></li><li><span data-t="note-2">Save stays disabled after editing</span></li><li><span data-t="note-3">History menu is empty on first launch</span></li><li>${words(10, 5)}</li></ul></div>`;
}

const KINDS = {
  terminal: (o) => `<div class="tbar dark"><span class="lights"><i></i><i></i><i></i></span><span class="ttl">${esc(o.title)}</span></div>${terminal(o)}`,
  browser, editor: (o) => `<div class="tbar dark"><span class="lights"><i></i><i></i><i></i></span><span class="ttl">${esc(o.title)}</span></div>${editor(o)}`,
  chat: (o) => `<div class="tbar slackbar"><span class="lights"><i></i><i></i><i></i></span><span class="ttl">${esc(o.title)}</span></div>${chat(o)}`,
  finder, notes,
};

const CSS = `
* { box-sizing: border-box; margin: 0; padding: 0; }
body { width: ${SW}px; height: ${SH}px; overflow: hidden; position: relative; font: 13px/1.45 -apple-system, "SF Pro Text", "Helvetica Neue", sans-serif; color: #1d1d1f;
  background: radial-gradient(1200px 700px at 15% 20%, #4d6fa8, transparent), radial-gradient(1000px 800px at 90% 90%, #9a4f7e, transparent), linear-gradient(160deg, #22345a, #3a2147); }
.menubar { position: absolute; left: 0; top: 0; width: 100%; height: ${MENU}px; background: rgba(28,28,34,.62); color: #fff; display: flex; align-items: center; padding: 0 14px; font-size: 13.5px; z-index: 900; }
.menubar span { padding: 3px 9px; border-radius: 5px; } .menubar .app { font-weight: 700; } .menubar .open { background: rgba(255,255,255,.25); }
.menubar .right { margin-left: auto; display: flex; gap: 4px; opacity: .92; }
.popup { position: absolute; background: rgba(236,236,240,.97); border-radius: 8px; box-shadow: 0 12px 40px rgba(0,0,0,.45); padding: 5px; z-index: 950; font-size: 13.5px; }
.popup div { padding: 3px 12px; border-radius: 4px; white-space: nowrap; } .popup .sep { height: 1px; background: #ccc; margin: 4px 6px; padding: 0; } .popup .hi { background: #2f6fec; color: #fff; }
.dock { position: absolute; bottom: 6px; left: 50%; transform: translateX(-50%); height: 64px; padding: 7px 10px; display: flex; gap: 8px; background: rgba(255,255,255,.2); border: 1px solid rgba(255,255,255,.3); border-radius: 20px; z-index: 880; }
.dock i { width: 50px; height: 50px; border-radius: 12px; display: block; }
.win { position: absolute; border-radius: 10px; overflow: hidden; box-shadow: 0 24px 70px rgba(0,0,0,.5), 0 0 0 .5px rgba(0,0,0,.6); background: #fff; display: flex; flex-direction: column; }
.win.full { border-radius: 0; box-shadow: none; }
.lights { display: inline-flex; gap: 8px; margin-right: 10px; } .lights i { width: 12px; height: 12px; border-radius: 50%; display: block; background: #ff5f57; } .lights i:nth-child(2) { background: #febc2e; } .lights i:nth-child(3) { background: #28c840; }
.tbar { height: 30px; display: flex; align-items: center; padding: 0 12px; background: #ececec; font-size: 12.5px; color: #555; flex: none; } .tbar .ttl { flex: 1; text-align: center; margin-right: 60px; }
.tbar.dark { background: #20212b; color: #9aa; } .tbar.slackbar { background: #3d1f40; color: #dcc; }
.dim { color: #8b8f99; } .small { font-size: 12px; }
.tui { background: #15161c; color: #d4d8e0; font: 13.5px/1.55 "SF Mono", Menlo, monospace; flex: 1; padding: 10px 18px; position: relative; overflow: hidden; }
.tui-head { display: flex; gap: 22px; color: #9aa3b2; border-bottom: 1px solid #2a2c36; padding-bottom: 8px; margin-bottom: 12px; } .tui-head b { color: #fab283; }
.msg { margin: 10px 0; max-width: 980px; } .msg.user { border-left: 3px solid #fab283; padding-left: 12px; color: #fff; }
.tool { border: 1px solid #2f3240; border-radius: 6px; margin: 10px 0; max-width: 980px; background: #1b1c24; } .tool-h { padding: 6px 12px; border-bottom: 1px solid #2f3240; color: #8fb3ff; } .tool pre { padding: 8px 12px; font: inherit; white-space: pre; }
.ok { color: #7fd962; } .bad { color: #ff6b6b; } .add { color: #7fd962; background: #1d3320; display: inline-block; width: 100%; } .del { color: #ff8a8a; background: #3a1f22; display: inline-block; width: 100%; }
code { font-family: "SF Mono", Menlo, monospace; font-size: .92em; background: rgba(127,127,127,.15); padding: 0 4px; border-radius: 4px; }
.tui-input { position: absolute; left: 18px; right: 18px; bottom: 34px; border: 1px solid #3a3d4c; border-radius: 6px; padding: 10px 14px; color: #fff; } .caret { color: #fab283; } .cursor-blk { display: inline-block; width: 9px; height: 17px; background: #d4d8e0; vertical-align: -3px; margin-left: 3px; }
.tui-foot { position: absolute; left: 18px; right: 18px; bottom: 8px; display: flex; justify-content: space-between; color: #fab283; font-size: 12px; }
.bbar { height: 38px; background: #e9e9ec; display: flex; align-items: center; padding: 0 12px; flex: none; } .tabs { display: flex; gap: 4px; } .tab { padding: 5px 14px; border-radius: 7px; font-size: 12.5px; color: #555; } .tab.on { background: #fff; color: #111; box-shadow: 0 1px 2px rgba(0,0,0,.12); }
.burl { height: 36px; background: #f6f6f8; border-bottom: 1px solid #ddd; display: flex; gap: 14px; align-items: center; padding: 0 14px; color: #666; flex: none; } .url { flex: 1; background: #fff; border-radius: 7px; padding: 4px 12px; color: #333; border: 1px solid #e3e3e6; }
.bcontent { flex: 1; overflow: hidden; position: relative; background: #fafafa; }
.app { display: flex; height: 100%; } .nav { width: 200px; background: #f1f1f4; border-right: 1px solid #e3e3e8; padding: 16px 10px; flex: none; } .nav div { padding: 7px 12px; border-radius: 7px; color: #444; } .nav .on { background: #fff; color: #111; font-weight: 600; box-shadow: 0 1px 2px rgba(0,0,0,.08); } .nav .logo { font-weight: 800; color: #111; margin-bottom: 12px; }
.main { flex: 1; padding: 0 30px 30px; position: relative; overflow: hidden; }
.topbar { height: 54px; display: flex; align-items: center; gap: 18px; border-bottom: 1px solid #eee; margin: 0 -30px 18px; padding: 0 30px; position: relative; } .crumb { color: #666; flex: 1; } .search { background: #f0f0f3; border-radius: 7px; padding: 5px 40px 5px 12px; color: #999; font-size: 12px; }
.bell { position: relative; font-size: 17px; } .badge { position: absolute; right: -9px; top: -6px; background: #e5484d; color: #fff; font-size: 10px; min-width: 16px; height: 16px; border-radius: 8px; display: grid; place-items: center; }
.avatar { width: 30px; height: 30px; border-radius: 50%; background: #6e56cf; color: #fff; display: grid; place-items: center; font-size: 12px; font-weight: 700; }
.notif { position: absolute; right: 58px; top: 48px; width: 330px; background: #fff; border-radius: 10px; box-shadow: 0 12px 40px rgba(0,0,0,.22); padding: 12px; z-index: 5; } .n-item { padding: 8px 4px; border-top: 1px solid #f0f0f0; font-size: 12.5px; }
.main h1 { font-size: 26px; margin-bottom: 16px; }
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(330px, 1fr)); gap: 16px; } .card { background: #fff; border: 1px solid #e8e8ec; border-radius: 12px; padding: 14px 18px; } .card.wide { grid-column: span 2; } .card h3 { font-size: 14px; margin-bottom: 8px; }
.row { display: flex; justify-content: space-between; padding: 7px 0; border-top: 1px solid #f2f2f2; color: #444; } .pill { background: #eef2ff; color: #4053d6; border-radius: 99px; padding: 0 10px; font-size: 12px; }
.bars { height: 110px; display: flex; align-items: flex-end; gap: 6px; margin: 6px 0 8px; } .bars i { flex: 1; background: #8da2fb; border-radius: 3px 3px 0 0; display: block; }
.tr { display: grid; grid-template-columns: 1.1fr 1fr .9fr .7fr; padding: 7px 0; border-top: 1px solid #f2f2f2; font-size: 12.5px; color: #444; } .tr.fail span:last-child { color: #e5484d; font-weight: 600; }
.slider { height: 6px; background: #e3e3e8; border-radius: 3px; position: relative; margin: 14px 0 8px; } .slider i { position: absolute; top: -7px; width: 20px; height: 20px; border-radius: 50%; background: #fff; box-shadow: 0 1px 4px rgba(0,0,0,.3); }
.actions { display: flex; justify-content: flex-end; gap: 10px; margin-top: 18px; } .btn { background: #111; color: #fff; padding: 8px 18px; border-radius: 8px; font-weight: 600; } .btn.ghost { background: #fff; color: #333; border: 1px solid #ddd; }
.toast { position: absolute; right: 30px; bottom: 26px; background: #2b1416; color: #ffb4b4; padding: 12px 18px; border-radius: 10px; box-shadow: 0 8px 24px rgba(0,0,0,.3); }
.docs { display: flex; height: 100%; } .toc { width: 220px; padding: 24px 18px; border-right: 1px solid #eee; color: #555; flex: none; } .toc div { padding: 4px 0; } .article { padding: 28px 54px; max-width: 860px; font-size: 15px; line-height: 1.65; color: #333; }
.article h1 { font-size: 32px; margin-bottom: 14px; } .article h2 { font-size: 21px; margin: 26px 0 10px; color: #111; } .article p { margin: 8px 0; } .article a { color: #2f6fec; } .article pre.code { background: #16171d; color: #d4d8e0; border-radius: 8px; padding: 12px 16px; font: 13px/1.5 "SF Mono", Menlo, monospace; margin: 10px 0; white-space: pre; }
.ed { display: flex; flex: 1; background: #1e1f26; color: #d6d6dc; overflow: hidden; } .tree { width: 220px; background: #191a20; padding: 12px 10px; font: 13px/1.9 -apple-system, sans-serif; color: #a9adb8; flex: none; } .tree .on { color: #fff; background: #2a2c37; border-radius: 5px; }
.code { flex: 1; font: 13.5px/1.75 "SF Mono", Menlo, monospace; position: relative; } .etabs { display: flex; background: #191a20; height: 34px; align-items: center; } .etabs span { padding: 7px 16px; color: #888; font-family: -apple-system, sans-serif; font-size: 12.5px; } .etabs .on { color: #fff; background: #1e1f26; }
.ln { white-space: pre; padding-left: 12px; } .ln.hl { background: #3a2e18; } .no { display: inline-block; width: 38px; color: #555a66; text-align: right; margin-right: 18px; }
.kw { color: #c792ea; } .fn { color: #82aaff; } .str { color: #c3e88d; } .num { color: #f78c6c; } .diag { color: #ffcb6b; margin-left: 24px; font-family: -apple-system, sans-serif; font-size: 12.5px; }
.status { position: absolute; bottom: 0; left: 0; right: 0; height: 26px; background: #16171c; color: #ffcb6b; font: 12px/26px -apple-system, sans-serif; padding-left: 14px; }
.slack { display: flex; flex: 1; overflow: hidden; } .ws { width: 230px; background: #3f0e40; color: #cfc3cf; padding: 14px 12px; flex: none; line-height: 2; } .ws b { color: #fff; font-size: 16px; } .ws .on { background: #1164a3; color: #fff; border-radius: 6px; padding-left: 8px; }
.chan { flex: 1; background: #fff; padding: 0 20px; position: relative; } .chead { height: 50px; display: flex; align-items: center; gap: 8px; font-weight: 700; border-bottom: 1px solid #eee; margin: 0 -20px 10px; padding: 0 20px; }
.cm { display: flex; gap: 10px; padding: 8px 0; font-size: 14px; } .av { width: 36px; height: 36px; border-radius: 8px; background: #e8a33d; color: #fff; display: grid; place-items: center; font-weight: 700; flex: none; } .composer { position: absolute; left: 20px; right: 20px; bottom: 18px; border: 1px solid #bbb; border-radius: 9px; padding: 12px; color: #999; }
.fbar { height: 52px; display: flex; gap: 16px; align-items: center; padding: 0 14px; background: #ececee; border-bottom: 1px solid #ddd; flex: none; } .views { margin-left: auto; letter-spacing: 6px; color: #555; font-size: 16px; }
.finder { display: flex; flex: 1; overflow: hidden; } .fside { width: 180px; background: #e8e8ea; padding: 12px; line-height: 2; color: #333; flex: none; } .fside .on { background: #d4d4d8; border-radius: 6px; padding-left: 6px; }
.fgrid { display: grid; grid-template-columns: repeat(4, 1fr); gap: 20px; padding: 24px; flex: 1; align-content: start; } .fgrid div { text-align: center; font-size: 12px; } .fgrid i { display: block; width: 64px; height: 56px; margin: 0 auto 6px; border-radius: 8px; background: linear-gradient(#7fb5ff, #3d82f6); }
.flist { flex: 1; } .flist div { display: grid; grid-template-columns: 2fr 1fr 1fr; padding: 6px 18px; font-size: 13px; border-bottom: 1px solid #f0f0f0; }
.notes { background: #fffdf2; flex: 1; padding: 18px 26px; } .notes h2 { font-size: 18px; margin-bottom: 8px; } .notes li { margin: 6px 0 6px 18px; font-size: 14px; }
`;

const DOCK_ICONS = ['#2d6cdf', '#1f1f24', 'linear-gradient(#5ac8fa,#007aff)', '#ffd60a', '#34c759', '#ff9f0a', '#af52de', '#ff375f', '#8e8e93', '#64d2ff', '#3f0e40'];
const MENUS = {
  Finder: ['File', 'Edit', 'View', 'Go', 'Window', 'Help'], Safari: ['File', 'Edit', 'View', 'History', 'Bookmarks', 'Window', 'Help'],
  Ghostty: ['File', 'Edit', 'View', 'Window', 'Help'], Zed: ['File', 'Edit', 'Selection', 'View', 'Go', 'Window', 'Help'],
  Slack: ['File', 'Edit', 'View', 'Go', 'Window', 'Help'], Notes: ['File', 'Edit', 'Format', 'View', 'Window', 'Help'],
};

function renderState(st, defs) {
  const order = st.order.slice().reverse();
  const wins = order.map((key, z) => {
    const w = st.wins[key], d = defs[key];
    return `<div class="win${w.full ? ' full' : ''}" data-win="${key}" style="left:${w.x}px;top:${w.y}px;width:${w.w}px;height:${w.h}px;z-index:${10 + z}">${KINDS[d.kind]({ ...d, ...w.opts })}</div>`;
  }).join('');
  const front = defs[st.order[0]];
  const items = MENUS[front.app] || ['File', 'Edit', 'View', 'Window', 'Help'];
  const bar = st.fullscreen ? '' : `<div class="menubar"><span style="font-size:15px"></span><span class="app" data-bar="app">${front.app}</span>${items.map((m) => `<span data-bar="${m}" class="${st.menu?.at === m ? 'open' : ''}">${m}</span>`).join('')}<span class="right"><span>⌁</span><span>◐</span><span>ᯤ</span><span>🔋</span><span>Thu 15 Oct  10:24</span></span></div>`;
  const popup = st.menu ? `<div class="popup" data-popup style="left:0;top:${MENU}px">${st.menu.items.map((it) => it === '-' ? '<div class="sep"></div>' : `<div data-item="${it}" class="${st.menu.hi === it ? 'hi' : ''}">${it}</div>`).join('')}</div>` : '';
  const dock = st.dock ? `<div class="dock">${DOCK_ICONS.map((c) => `<i style="background:${c}"></i>`).join('')}</div>` : '';
  return wins + bar + popup + dock;
}

// ---------------------------------------------------------------------------------------------
// Scenario builder
// ---------------------------------------------------------------------------------------------
class Scenario {
  constructor(name, description, duration) {
    Object.assign(this, { name, description, duration, defs: {}, states: [], acts: [], wordList: [], segments: [], spans: [], wholeSpec: null, keys: [] });
  }
  win(key, def) { this.defs[key] = def; return this; }
  /** The screen from `t` on: `fn` edits a copy of the previous state. */
  at(t, fn) {
    const prev = this.states.length ? structuredClone(this.states[this.states.length - 1].st) : { wins: {}, order: [], menu: null, dock: false, fullscreen: false };
    fn(prev);
    this.states.push({ t, st: prev });
    return this;
  }
  say(t, text, ms = 290) {
    const ws = text.split(' ');
    const t0 = t;
    for (const w of ws) { this.wordList.push({ t0: t, t1: t + ms - 40, text: w }); t += ms + (w.endsWith(',') ? 160 : 0) + (w.endsWith('.') ? 260 : 0); }
    this.segments.push({ t0, t1: t, text });
    return t;
  }
  circle(t, target, dur = 900) { this.acts.push({ type: 'circle', t, target, dur }); return t + dur; }
  underline(t, target, dur = 500) { this.acts.push({ type: 'underline', t, target, dur }); return t + dur; }
  arrow(t, from, to, dur = 700) { this.acts.push({ type: 'arrow', t, from, to, dur }); return t + dur + 450; }
  click(t, target, opts = {}) { this.acts.push({ type: 'click', t, target, ...opts }); return t; }
  drag(t, t1, from, to) { this.acts.push({ type: 'drag', t, t1, from, to }); return t1; }
  scroll(t, target, dy) { this.acts.push({ type: 'scroll', t, target, dy }); return t; }
  key(t, shortcut) { this.keys.push({ kind: 'key', t, shortcut }); return t; }
  /** The cursor rests near `target` (small drift) from t to t1. */
  rest(t, t1, target) { this.acts.push({ type: 'rest', t, t1, target }); return t1; }
  /** Stray motion through absolute points `[t, x, y]`. */
  wander(points) { this.acts.push({ type: 'wander', points }); return this; }
  truth(t0, t1, subject, ideal, targets = []) { this.spans.push({ t0, t1, subject, ideal, targets }); return this; }
  whole(ideal) { this.wholeSpec = ideal; return this; }
}

// Ideal/target specs, resolved against the rendered state at a time:
//   'web'            the window's rect          'web#card-account'  an element in a window
//   'bar#View'       a menu-bar item            'menu'  /  'menu#as List'  the open menu
//   { of: [...specs], pad: 24 }                 union, grown by pad (clipped to the screen)
//   { rect: [x, y, w, h] }                      literal points

// ---------------------------------------------------------------------------------------------
// The scenarios
// ---------------------------------------------------------------------------------------------
const FULLWIN = { x: 0, y: MENU, w: SW, h: SH - MENU };
const ALL = [];
const def = (name, description, duration, build) => ALL.push({ name, description, duration, build });

def('maximized-terminal', 'One maximized OpenCode terminal; ink on a failing test, then the prompt', 14_000, (s) => {
  s.win('term', { kind: 'terminal', app: 'Ghostty', bundleId: 'com.mitchellh.ghostty', title: 'opencode — ~/code/acme' });
  s.at(0, (st) => { st.wins.term = { ...FULLWIN }; st.order = ['term']; });
  s.rest(0, 1_500, 'term#tool');
  s.say(300, 'So OpenCode ran the settings tests,');
  s.circle(2_200, 'term#error', 950);
  s.say(2_400, 'and this one is failing, the header inset.');
  s.underline(5_200, 'term#expect');
  s.say(5_300, 'It expected zero and got minus twenty six.');
  s.rest(6_000, 8_400, 'term#expect');
  s.underline(9_000, 'term#prompt-text', 600);
  s.say(9_100, 'Now I want it to also check the slider here.');
  s.rest(10_000, 14_000, 'term#prompt-text');
  s.truth(0, 8_600, 'test', { of: ['term#tool'], pad: 36 }, ['term#error']);
  s.truth(8_600, 14_000, 'prompt', { of: ['term#prompt-text', 'term#diff'], pad: 40 }, ['term#prompt-text']);
  s.whole({ of: ['term#tool', 'term#prompt-text'], pad: 30 });
});

def('side-by-side', 'Editor and browser side by side; all the activity is in the browser', 13_000, (s) => {
  s.win('ed', { kind: 'editor', app: 'Zed', bundleId: 'dev.zed.Zed', title: 'AccountCard.tsx — acme' });
  s.win('web', { kind: 'browser', app: 'Safari', bundleId: 'com.apple.Safari', title: 'Settings · acme' });
  s.at(0, (st) => { st.wins.ed = { x: 0, y: MENU, w: 864, h: SH - MENU }; st.wins.web = { x: 864, y: MENU, w: 864, h: SH - MENU }; st.order = ['ed', 'web']; });
  s.rest(0, 1_200, 'ed#code-7');
  s.say(400, 'Okay, in the browser,');
  s.click(2_000, 'web#save');
  s.at(2_050, (st) => { st.order = ['web', 'ed']; st.wins.web.opts = { toast: true }; });
  s.say(2_100, 'when I hit save it just fails,');
  s.circle(4_200, 'web#toast', 900);
  s.say(4_300, 'this toast, every single time.');
  s.rest(5_300, 7_000, 'web#toast');
  s.circle(7_600, 'web#card-account', 1000);
  s.say(7_700, 'And the account card header is still clipped.');
  s.rest(8_800, 13_000, 'web#card-account');
  s.truth(0, 13_000, 'web', 'web', []);
  s.whole('web');
});

def('arrow-across', 'Ink in the browser, then an arrow from the browser to the terminal', 13_000, (s) => {
  s.win('term', { kind: 'terminal', app: 'Ghostty', bundleId: 'com.mitchellh.ghostty', title: 'opencode — ~/code/acme' });
  s.win('web', { kind: 'browser', app: 'Safari', bundleId: 'com.apple.Safari', title: 'Settings · acme' });
  s.at(0, (st) => { st.wins.term = { x: 30, y: 70, w: 820, h: 760 }; st.wins.web = { x: 800, y: 120, w: 900, h: 820 }; st.order = ['web', 'term']; st.dock = true; });
  s.say(300, 'Look at the account card in the browser,');
  s.circle(1_800, 'web#card-head', 800);
  s.say(2_700, 'the header sits too high.');
  s.rest(2_600, 4_800, 'web#card-head');
  s.arrow(5_400, 'web#card-account', 'term#expect', 800);
  s.say(5_500, 'That is exactly this failure in the terminal,');
  s.rest(6_800, 9_000, 'term#expect');
  s.say(9_200, 'so the test is right and the CSS is wrong.');
  s.rest(9_000, 13_000, 'term#expect');
  s.truth(0, 5_200, 'card', 'web', ['web#card-head']);
  s.truth(5_200, 13_000, 'link', { of: ['web#card-account', 'term#tool'], pad: 40 }, ['web#card-account', 'term#expect']);
  s.whole({ of: ['web', 'term'], pad: 8 });
});

def('two-windows', 'Comparing two small windows: ink in Notes, ink in Finder, back and forth', 13_000, (s) => {
  s.win('notes', { kind: 'notes', app: 'Notes', bundleId: 'com.apple.Notes', title: 'Bugs' });
  s.win('fin', { kind: 'finder', app: 'Finder', bundleId: 'com.apple.finder', title: 'Downloads' });
  s.at(0, (st) => { st.wins.notes = { x: 170, y: 230, w: 540, h: 440 }; st.wins.fin = { x: 760, y: 260, w: 760, h: 500, opts: { list: true } }; st.order = ['fin', 'notes']; st.dock = true; });
  s.rest(0, 1_200, 'notes#note-3');
  s.say(300, 'My note says the history menu is empty,');
  s.circle(1_600, 'notes#note-3', 900);
  s.rest(2_500, 4_200, 'notes#note-3');
  s.say(4_400, 'but the recording I made of it is right here,');
  s.circle(4_800, 'fin#file-4', 800);
  s.rest(5_600, 7_400, 'fin#file-4');
  s.arrow(7_800, 'fin#file-4', 'notes#note-3', 700);
  s.say(7_900, 'so attach this file to that note.');
  s.rest(9_000, 13_000, 'notes#note-3');
  s.truth(0, 4_300, 'notes', 'notes', ['notes#note-3']);
  s.truth(4_300, 7_600, 'finder', 'fin', ['fin#file-4']);
  s.truth(7_600, 13_000, 'both', { of: ['notes', 'fin'], pad: 8 }, ['notes#note-3', 'fin#file-4']);
  s.whole({ of: ['notes', 'fin'], pad: 8 });
});

def('tiny-badge', 'A tiny notification badge in a huge browser window', 12_000, (s) => {
  s.win('web', { kind: 'browser', app: 'Safari', bundleId: 'com.apple.Safari', title: 'Overview · acme' });
  s.at(0, (st) => { st.wins.web = { x: 24, y: 48, w: 1680, h: 1050, opts: { tab: 'Overview' } }; st.order = ['web']; });
  s.rest(0, 1_200, 'web#chart');
  s.say(300, 'See this little badge up here,');
  s.circle(1_800, 'web#badge', 700);
  s.say(2_600, 'it says three but there is nothing new.');
  s.rest(2_500, 4_600, 'web#badge');
  s.click(5_000, 'web#bell');
  s.at(5_080, (st) => { st.wins.web.opts = { ...st.wins.web.opts, notifications: true }; });
  s.say(5_200, 'When I open it, these are all old.');
  s.rest(5_400, 8_000, 'web#notifs');
  s.underline(8_400, 'web#n-0', 500);
  s.say(8_500, 'All three of these were read yesterday.');
  s.rest(9_000, 12_000, 'web#notifs');
  s.truth(0, 4_900, 'badge', { of: ['web#badge', 'web#bell'], pad: 200 }, ['web#badge']);
  s.truth(4_900, 12_000, 'notifs', { of: ['web#notifs', 'web#bell'], pad: 60 }, ['web#notifs']);
  s.whole({ of: ['web#notifs', 'web#badge'], pad: 80 });
});

def('menu-bar', 'Opening the View menu from the menu bar and switching Finder to a list', 12_000, (s) => {
  s.win('fin', { kind: 'finder', app: 'Finder', bundleId: 'com.apple.finder', title: 'Downloads' });
  s.win('slack', { kind: 'chat', app: 'Slack', bundleId: 'com.tinyspeck.slackmacgap', title: 'deploys — Anomaly' });
  s.at(0, (st) => { st.wins.slack = { x: 860, y: 90, w: 820, h: 900 }; st.wins.fin = { x: 160, y: 130, w: 900, h: 600 }; st.order = ['fin', 'slack']; st.dock = true; });
  s.rest(0, 1_400, 'fin#file-2');
  s.say(300, 'I want Finder to show a list,');
  s.click(2_400, 'bar#View');
  s.at(2_460, (st) => { st.menu = { at: 'View', items: ['as Icons', 'as List', 'as Columns', 'as Gallery', '-', 'Use Groups', 'Sort By', 'Clean Up', '-', 'Show Path Bar', 'Show Status Bar', 'Hide Sidebar', '-', 'Show Toolbar', 'Customize Toolbar…'] }; });
  s.say(2_600, 'so I go to View,');
  s.rest(2_700, 3_900, 'menu#as List');
  s.at(3_400, (st) => { st.menu.hi = 'as List'; });
  s.click(4_400, 'menu#as List');
  s.at(4_460, (st) => { st.menu = null; st.wins.fin.opts = { list: true }; });
  s.say(4_700, 'pick as List, and now the sizes show up.');
  s.rest(5_000, 7_400, 'fin#file-4');
  s.circle(7_800, 'fin#file-4', 800);
  s.say(7_900, 'But this recording has no date.');
  s.rest(8_700, 12_000, 'fin#file-4');
  s.truth(0, 2_400, 'finder', 'fin', []);
  s.truth(2_400, 4_700, 'menu', { of: ['fin', 'menu', 'bar#View'], pad: 8 }, []);
  s.truth(4_700, 12_000, 'finder', 'fin', []);
  s.whole({ of: ['fin', 'bar#View'], pad: 8 });
});

def('talk-only', 'Talking over Slack, then switching to the browser with the keyboard and talking', 14_000, (s) => {
  s.win('slack', { kind: 'chat', app: 'Slack', bundleId: 'com.tinyspeck.slackmacgap', title: 'deploys — Anomaly' });
  s.win('web', { kind: 'browser', app: 'Safari', bundleId: 'com.apple.Safari', title: 'Deployments · acme' });
  s.at(0, (st) => { st.wins.web = { x: 360, y: 120, w: 1100, h: 820, opts: { tab: 'Deployments' } }; st.wins.slack = { x: 200, y: 70, w: 1000, h: 760 }; st.order = ['slack', 'web']; });
  s.rest(0, 6_000, 'slack#msg-0');
  s.say(400, 'Mara says the deploy failed again and she is right,');
  s.say(3_600, 'it is the same test as yesterday.');
  s.key(6_300, '⌘⇥');
  s.at(6_400, (st) => { st.order = ['web', 'slack']; });
  s.say(6_700, 'Look, the deployments page shows it failing on main.');
  s.rest(6_400, 14_000, 'web#row-3');
  s.say(10_600, 'Now everything after it went through fine.');
  s.truth(0, 6_350, 'slack', 'slack', []);
  s.truth(6_350, 14_000, 'web', 'web', []);
  s.whole({ of: ['slack', 'web'], pad: 8 });
});

def('window-drag', 'Dragging the Notes window across the screen, then inking in it', 13_000, (s) => {
  s.win('notes', { kind: 'notes', app: 'Notes', bundleId: 'com.apple.Notes', title: 'Bugs' });
  s.win('web', { kind: 'browser', app: 'Safari', bundleId: 'com.apple.Safari', title: 'Settings · acme' });
  s.at(0, (st) => { st.wins.web = { x: 520, y: 60, w: 1180, h: 960 }; st.wins.notes = { x: 60, y: 160, w: 520, h: 420 }; st.order = ['notes', 'web']; });
  s.say(300, 'Let me move my notes out of the way,');
  const path = [[60, 160], [180, 210], [420, 330], [640, 450], [760, 520], [800, 540]];
  s.drag(2_000, 3_300, { notesTitle: [60 + 260, 160 + 15] }, { notesTitle: [800 + 260, 540 + 15] });
  path.forEach(([x, y], i) => s.at(2_000 + i * 260, (st) => { st.wins.notes.x = x; st.wins.notes.y = y; }));
  s.say(3_600, 'and the second bug here is the slider one.');
  s.circle(4_400, 'notes#note-1', 800);
  s.rest(5_200, 8_000, 'notes#note-1');
  s.underline(8_400, 'notes#note-2', 500);
  s.say(8_500, 'And this one I fixed this morning.');
  s.rest(9_000, 13_000, 'notes#note-2');
  s.truth(0, 1_900, 'notes', 'notes', []);
  s.truth(1_900, 3_900, 'move', { rect: [60, 160, 800 + 520 - 60, 540 + 420 - 160] }, []);
  s.truth(3_900, 13_000, 'notes', 'notes', ['notes#note-1']);
  s.whole({ rect: [60, 160, 800 + 520 - 60, 540 + 420 - 160] });
});

def('overlapping-front', 'Three overlapping windows; all the activity is on the front one', 12_000, (s) => {
  s.win('ed', { kind: 'editor', app: 'Zed', bundleId: 'dev.zed.Zed', title: 'AccountCard.tsx — acme' });
  s.win('term', { kind: 'terminal', app: 'Ghostty', bundleId: 'com.mitchellh.ghostty', title: 'opencode — ~/code/acme' });
  s.win('slack', { kind: 'chat', app: 'Slack', bundleId: 'com.tinyspeck.slackmacgap', title: 'deploys — Anomaly' });
  s.at(0, (st) => { st.wins.slack = { x: 80, y: 80, w: 900, h: 700 }; st.wins.term = { x: 300, y: 180, w: 1000, h: 760 }; st.wins.ed = { x: 640, y: 260, w: 1000, h: 780 }; st.order = ['ed', 'term', 'slack']; });
  s.rest(0, 1_600, 'ed#tab');
  s.say(300, 'In the editor, line eight,');
  s.circle(1_900, 'ed#diag', 900);
  s.say(2_100, 'the warning says the negative margin pulls the header out.');
  s.rest(2_900, 5_600, 'ed#code-7');
  s.click(6_000, 'ed#code-7');
  s.say(6_200, 'I will just delete that style.');
  s.underline(8_300, 'ed#status-text', 500);
  s.say(8_300, 'And the status bar still shows one warning.');
  s.rest(8_900, 12_000, 'ed#status-text');
  s.truth(0, 12_000, 'editor', 'ed', []);
  s.whole('ed');
});

def('three-window-hop', 'Thirty seconds hopping between editor, browser and terminal', 30_000, (s) => {
  s.win('ed', { kind: 'editor', app: 'Zed', bundleId: 'dev.zed.Zed', title: 'AccountCard.tsx — acme' });
  s.win('web', { kind: 'browser', app: 'Safari', bundleId: 'com.apple.Safari', title: 'Settings · acme' });
  s.win('term', { kind: 'terminal', app: 'Ghostty', bundleId: 'com.mitchellh.ghostty', title: 'opencode — ~/code/acme' });
  s.at(0, (st) => { st.wins.ed = { x: 0, y: MENU, w: 860, h: 560 }; st.wins.term = { x: 0, y: MENU + 560, w: 860, h: SH - MENU - 560 }; st.wins.web = { x: 860, y: MENU, w: 868, h: SH - MENU }; st.order = ['ed', 'web', 'term']; });
  s.rest(0, 1_200, 'ed#tab');
  s.say(300, 'First the component, this margin,');
  s.circle(1_800, 'ed#code-7', 900);
  s.rest(2_700, 6_500, 'ed#code-7');
  s.say(3_200, 'is the bug everyone is hitting.');
  s.click(8_200, 'web#nav-Settings');
  s.at(8_260, (st) => { st.order = ['web', 'ed', 'term']; });
  s.say(8_400, 'In the app the card looks like this,');
  s.circle(10_000, 'web#card-account', 1000);
  s.rest(11_000, 15_000, 'web#card-account');
  s.say(11_200, 'the header is half outside it.');
  s.click(17_200, 'term#prompt');
  s.at(17_260, (st) => { st.order = ['term', 'web', 'ed']; });
  s.say(17_400, 'And OpenCode already found it,');
  s.underline(19_000, 'term#expect', 500);
  s.rest(19_500, 23_000, 'term#expect');
  s.say(19_600, 'expected zero got minus twenty six.');
  s.click(25_000, 'ed#code-2');
  s.at(25_060, (st) => { st.order = ['ed', 'term', 'web']; });
  s.say(25_200, 'So back here, I will take the fix.');
  s.rest(25_300, 30_000, 'ed#code-2');
  s.truth(0, 8_100, 'editor', 'ed', []);
  s.truth(8_100, 17_100, 'web', 'web', []);
  s.truth(17_100, 24_900, 'term', 'term', []);
  s.truth(24_900, 30_000, 'editor', 'ed', []);
  s.whole({ rect: [0, MENU, SW, SH - MENU] });
});

def('stray-cursor', 'Ink on a chart while the cursor strays to far corners and another window', 13_000, (s) => {
  s.win('web', { kind: 'browser', app: 'Safari', bundleId: 'com.apple.Safari', title: 'Overview · acme' });
  s.win('slack', { kind: 'chat', app: 'Slack', bundleId: 'com.tinyspeck.slackmacgap', title: 'deploys — Anomaly' });
  s.at(0, (st) => { st.wins.slack = { x: 1020, y: 380, w: 690, h: 700 }; st.wins.web = { x: 40, y: 60, w: 1100, h: 820, opts: { tab: 'Overview' } }; st.order = ['web', 'slack']; st.dock = true; });
  s.rest(0, 1_000, 'web#chart');
  s.say(300, 'This spike at eleven,');
  s.circle(1_400, 'web#spike', 700);
  s.say(2_300, 'nobody knows where it comes from.');
  s.wander([[2_400, 700, 420], [3_300, 1660, 1060], [4_600, 1690, 1080], [5_200, 1300, 700], [5_900, 1500, 500], [6_600, 600, 380]]);
  s.rest(6_700, 8_200, 'web#spike');
  s.underline(8_600, 'web#peak', 500);
  s.say(8_600, 'The whole chart is normal apart from it.');
  s.wander([[9_200, 400, 700], [10_100, 20, 1100], [10_900, 30, 1090], [11_600, 500, 500]]);
  s.rest(11_700, 13_000, 'web#chart');
  s.truth(0, 13_000, 'chart', 'web', ['web#chart']);
  s.whole('web');
});

def('fullscreen-app', 'Zed in macOS full screen; ink on a line and its diagnostic', 12_000, (s) => {
  s.win('ed', { kind: 'editor', app: 'Zed', bundleId: 'dev.zed.Zed', title: 'AccountCard.tsx — acme' });
  s.at(0, (st) => { st.wins.ed = { x: 0, y: 0, w: SW, h: SH, full: true }; st.order = ['ed']; st.fullscreen = true; });
  s.rest(0, 1_500, 'ed#code-2');
  s.say(300, 'Full screen in Zed now,');
  s.underline(1_900, 'ed#code-7', 600);
  s.say(2_000, 'this style on line eight');
  s.circle(3_600, 'ed#diag', 900);
  s.say(3_800, 'and the warning right next to it.');
  s.rest(4_500, 8_200, 'ed#diag');
  s.underline(8_600, 'ed#status-text', 500);
  s.say(8_700, 'The status bar agrees, one warning.');
  s.rest(9_100, 12_000, 'ed#status-text');
  s.truth(0, 8_400, 'line', { of: ['ed#code-2', 'ed#code-7', 'ed#diag'], pad: 60 }, ['ed#code-7', 'ed#diag']);
  s.truth(8_400, 12_000, 'status', { of: ['ed#status-text'], pad: 140 }, ['ed#status-text']);
  s.whole({ of: ['ed#code-2', 'ed#diag', 'ed#status-text'], pad: 40 });
});

def('scrolling-page', 'Scrolling a long docs page in a big browser window, then inking a paragraph', 15_000, (s) => {
  s.win('web', { kind: 'browser', app: 'Safari', bundleId: 'com.apple.Safari', title: 'API reference · acme', url: 'docs.acme.dev/api' });
  s.at(0, (st) => { st.wins.web = { x: 150, y: 44, w: 1420, h: 1060, opts: { page: 'docs', scrollTo: 'h-3', scrollFrac: 0 } }; st.order = ['web']; });
  s.rest(0, 1_000, 'web#para-0');
  s.say(300, 'Looking for the rate limit section,');
  [1_400, 2_000, 2_600, 3_200].forEach((t, i) => { s.scroll(t, 'web#view', -120); s.at(t + 40, (st) => { st.wins.web.opts = { ...st.wins.web.opts, scrollFrac: (i + 1) / 4 }; }); });
  s.rest(3_400, 4_800, 'web#para-3');
  s.say(3_600, 'here it is,');
  s.circle(5_000, 'web#code-429', 800);
  s.say(5_100, 'it says six hundred a minute returns a four twenty nine.');
  s.rest(5_800, 8_600, 'web#code-429');
  s.click(9_000, 'web#link-3');
  s.say(9_000, 'and the read more link');
  s.underline(10_200, 'web#rule', 600);
  s.say(10_300, 'goes nowhere, but this paragraph is what I need.');
  s.rest(10_800, 15_000, 'web#para-3');
  s.truth(0, 4_900, 'scroll', { of: ['web#view'] }, []);
  s.truth(4_900, 15_000, 'para', { of: ['web#para-3', 'web#h-3', 'web#link-3'], pad: 40 }, ['web#code-429']);
  s.whole({ of: ['web#para-3', 'web#h-3', 'web#link-3'], pad: 40 });
});

// ---------------------------------------------------------------------------------------------
// Running a scenario: render states, resolve targets, synthesise input
// ---------------------------------------------------------------------------------------------
async function loadPlaywright() {
  try { return await import('playwright'); } catch {}
  const dir = new URL('../../node_modules/playwright/index.mjs', import.meta.url).pathname;
  if (existsSync(dir)) return await import(dir);
  throw new Error('Playwright not found: install it, or run from a checkout with public/site/node_modules');
}

const clampPt = ([x, y]) => [Math.max(0, Math.min(SW - 1, x)), Math.max(0, Math.min(SH - 1, y))];
const rectUnion = (a, b) => { const x0 = Math.min(a[0], b[0]), y0 = Math.min(a[1], b[1]); return [x0, y0, Math.max(a[0] + a[2], b[0] + b[2]) - x0, Math.max(a[1] + a[3], b[1] + b[3]) - y0]; };
const clipRect = ([x, y, w, h]) => { const x0 = Math.max(0, x), y0 = Math.max(0, y); return [x0, y0, Math.min(SW, x + w) - x0, Math.min(SH, y + h) - y0]; };
const center = (r) => [r[0] + r[2] / 2, r[1] + r[3] / 2];

async function run(page, sc) {
  const dir = join(OUT, sc.name);
  mkdirSync(join(dir, 'states'), { recursive: true });
  seed = [...sc.name].reduce((a, c) => a * 31 + c.charCodeAt(0), 7) >>> 0;
  sc.states.sort((a, b) => a.t - b.t);
  const geo = [];
  for (let i = 0; i < sc.states.length; i++) {
    const { st } = sc.states[i];
    await page.evaluate((html) => {
      document.body.innerHTML = html;
      for (const a of document.querySelectorAll('[data-scroll-to]')) {
        const el = a.dataset.scrollTo && a.querySelector(`[data-t="${a.dataset.scrollTo}"]`);
        const dy = el ? (el.getBoundingClientRect().y - a.parentElement.getBoundingClientRect().y - 90) * +a.dataset.scrollFrac : 0;
        a.style.transform = `translateY(${-dy}px)`;
      }
    }, renderState(st, sc.defs));
    geo.push(await page.evaluate(() => {
      const out = {};
      const r = (el) => { const b = el.getBoundingClientRect(); return [b.x, b.y, b.width, b.height]; };
      // Elements are clipped to their window and any scrolling viewport, like what's visible.
      const clip = (a, b) => { const x0 = Math.max(a[0], b[0]), y0 = Math.max(a[1], b[1]); return [x0, y0, Math.max(0, Math.min(a[0] + a[2], b[0] + b[2]) - x0), Math.max(0, Math.min(a[1] + a[3], b[1] + b[3]) - y0)]; };
      for (const w of document.querySelectorAll('[data-win]')) {
        const key = w.dataset.win;
        out[key] = r(w);
        for (const el of w.querySelectorAll('[data-t]')) {
          let b = clip(r(el), out[key]);
          const view = el.closest('.bcontent');
          if (view && view !== el) b = clip(b, r(view));
          if (b[2] > 0 && b[3] > 0) out[`${key}#${el.dataset.t}`] = b;
        }
      }
      for (const el of document.querySelectorAll('[data-bar]')) out[`bar#${el.dataset.bar}`] = r(el);
      const popup = document.querySelector('[data-popup]');
      if (popup) {
        const bar = document.querySelector('.menubar .open');
        if (bar) { popup.style.left = bar.getBoundingClientRect().x + 'px'; }
        out.menu = r(popup);
        for (const el of popup.querySelectorAll('[data-item]')) out[`menu#${el.dataset.item}`] = r(el);
      }
      return out;
    }));
    await page.screenshot({ path: join(dir, 'states', `${String(i).padStart(2, '0')}.png`) });
  }
  const stateAt = (t) => { let i = 0; sc.states.forEach((s, j) => { if (s.t <= t) i = j; }); return i; };
  const look = (spec, t) => {
    const g = geo[stateAt(t)];
    if (typeof spec === 'string') { if (!g[spec]) throw new Error(`${sc.name}: no ${spec} at ${t}`); return clipRect(g[spec]); }
    if (spec.rect) return spec.rect;
    if (spec.notesTitle) return [spec.notesTitle[0] - 2, spec.notesTitle[1] - 2, 4, 4];
    const u = spec.of.map((x) => look(x, t)).reduce(rectUnion);
    const p = spec.pad || 0;
    return clipRect([u[0] - p, u[1] - p, u[2] + 2 * p, u[3] + 2 * p]);
  };

  // Ink, clicks, drags and scrolls, plus where the cursor has to be when.
  const strokes = [], events = [], path = [];
  const norm = ([x, y]) => [+(x / SW).toFixed(4), +(y / SH).toFixed(4)];
  const stroke = (pts, t0, dur) => {
    const n = pts.length, flat = [];
    pts.forEach(([x, y], i) => { const [cx, cy] = clampPt([x, y]); flat.push(Math.round(cx / SW * 10000), Math.round(cy / SH * 10000), Math.round(i / (n - 1) * dur)); path.push([t0 + i / (n - 1) * dur, cx, cy, 'ink']); });
    strokes.push({ t0, t1: t0 + dur, fw: SW, fh: SH, pts: flat });
  };
  for (const a of sc.acts) {
    if (a.type === 'circle') {
      const r = look(a.target, a.t), [cx, cy] = center(r);
      const rx = r[2] / 2 + 14 + rand() * 8, ry = r[3] / 2 + 12 + rand() * 6, n = 46, start = -2.3 + jitter(0.3), sweep = Math.PI * 2 + 0.35;
      const pts = Array.from({ length: n }, (_, i) => { const u = i / (n - 1), ang = start + sweep * u, wob = 1 + 0.05 * Math.sin(u * 9 + seed % 7) + jitter(0.012); return [cx + rx * wob * Math.cos(ang) + u * 6, cy + ry * wob * Math.sin(ang)]; });
      stroke(pts, a.t, a.dur);
    } else if (a.type === 'underline') {
      const r = look(a.target, a.t), y = r[1] + r[3] + 5, n = 18;
      stroke(Array.from({ length: n }, (_, i) => [r[0] - 6 + (r[2] + 12) * i / (n - 1), y + jitter(1.5) + i * 0.15]), a.t, a.dur);
    } else if (a.type === 'arrow') {
      const A = center(look(a.from, a.t)), B = look(a.to, a.t), Bc = [B[0] + B[2] + 14, B[1] + B[3] / 2];
      const n = 26, bend = 60;
      const shaft = Array.from({ length: n }, (_, i) => { const u = i / (n - 1); return [A[0] + (Bc[0] - A[0]) * u, A[1] + (Bc[1] - A[1]) * u - bend * Math.sin(u * Math.PI) + jitter(1)]; });
      stroke(shaft, a.t, a.dur);
      const ang = Math.atan2(Bc[1] - shaft[n - 3][1], Bc[0] - shaft[n - 3][0]);
      const head = [[Bc[0] - 26 * Math.cos(ang - 0.5), Bc[1] - 26 * Math.sin(ang - 0.5)], Bc, [Bc[0] - 26 * Math.cos(ang + 0.5), Bc[1] - 26 * Math.sin(ang + 0.5)]];
      stroke(head, a.t + a.dur + 250, 200);
    } else if (a.type === 'click') {
      const r = look(a.target, a.t), p = [r[0] + r[2] * (0.4 + rand() * 0.2), r[1] + r[3] * (0.4 + rand() * 0.2)];
      events.push({ kind: 'click', t: a.t, x: norm(p)[0], y: norm(p)[1], button: 'left' });
      path.push([a.t - 120, ...p, 'hold'], [a.t + 120, ...p, 'hold']);
    } else if (a.type === 'drag') {
      const p0 = center(look(a.from, a.t)), p1 = center(look(a.to, a.t1));
      events.push({ kind: 'drag', t: a.t, t1: a.t1, x0: norm(p0)[0], y0: norm(p0)[1], x1: norm(p1)[0], y1: norm(p1)[1] });
      for (let i = 0; i <= 10; i++) { const u = i / 10, e = u * u * (3 - 2 * u); path.push([a.t + (a.t1 - a.t) * u, p0[0] + (p1[0] - p0[0]) * e, p0[1] + (p1[1] - p0[1]) * e, 'ink']); }
    } else if (a.type === 'scroll') {
      const p = center(look(a.target, a.t));
      events.push({ kind: 'scroll', t: a.t, x: norm(p)[0], y: norm(p)[1], dx: 0, dy: a.dy });
      path.push([a.t - 80, ...p, 'hold'], [a.t + 80, ...p, 'hold']);
    } else if (a.type === 'rest') {
      const r = look(a.target, a.t), p = [r[0] + r[2] * (0.3 + rand() * 0.4), r[1] + r[3] * (0.3 + rand() * 0.4)];
      for (let t = a.t; t <= a.t1; t += 700 + rand() * 900) path.push([t, p[0] + jitter(10), p[1] + jitter(6), 'hold']);
      path.push([a.t1, ...p, 'hold']);
    } else if (a.type === 'wander') {
      for (const [t, x, y] of a.points) path.push([t, x, y, 'hold']);
    }
  }
  path.sort((a, b) => a[0] - b[0]);
  // Cursor samples at 15 Hz: between waypoints the hand moves late and fast (minimum jerk, a
  // slight arc), then holds still; Apparent drops samples within 2 pt of the previous one.
  const cursor = [];
  let last = null;
  for (let t = 0; t <= sc.duration; t += 66) {
    let j = path.findIndex((p) => p[0] > t);
    let pos;
    if (j < 0) pos = path[path.length - 1].slice(1, 3);
    else if (j === 0) pos = path[0].slice(1, 3);
    else {
      const a = path[j - 1], b = path[j];
      if (b[3] === 'ink' && a[3] === 'ink') { const u = (t - a[0]) / (b[0] - a[0]); pos = [a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u]; }
      else {
        const dist = Math.hypot(b[1] - a[1], b[2] - a[2]), move = Math.min(b[0] - a[0], 260 + dist * 0.45), start = b[0] - move;
        const u = t < start ? 0 : (t - start) / move, e = u * u * u * (10 - 15 * u + 6 * u * u);
        const arc = Math.sin(u * Math.PI) * dist * 0.08;
        const nx = -(b[2] - a[2]) / (dist || 1), ny = (b[1] - a[1]) / (dist || 1);
        pos = [a[1] + (b[1] - a[1]) * e + nx * arc, a[2] + (b[2] - a[2]) * e + ny * arc];
      }
    }
    pos = clampPt(pos);
    if (!last || Math.hypot(pos[0] - last[0], pos[1] - last[1]) >= 2) { cursor.push([t, +(pos[0] / SW).toFixed(4), +(pos[1] / SH).toFixed(4)]); last = pos; }
  }

  // The window log: one snapshot per state (they only differ when the state does).
  const windows = sc.states.map(({ t, st }, i) => {
    const g = geo[i], out = [];
    if (st.menu) out.push({ id: 900, app: sc.defs[st.order[0]].app, bundleId: sc.defs[st.order[0]].bundleId, bounds: g.menu.map(Math.round), layer: 101 });
    st.order.forEach((key, z) => {
      const d = sc.defs[key];
      out.push({ id: 4100 + Object.keys(sc.defs).indexOf(key), app: d.app, bundleId: d.bundleId, title: d.title, bounds: g[key].map(Math.round), layer: 0, ...(z === 0 ? { frontmost: true } : {}) });
    });
    if (!st.fullscreen) out.push({ id: 6, app: 'Window Server', regular: false, title: 'Menubar', bounds: [0, 0, SW, MENU], layer: 24 });
    if (st.dock) out.push({ id: 38, app: 'Dock', bundleId: 'com.apple.dock', regular: false, bounds: [0, SH - DOCK_H, SW, DOCK_H], layer: 20 });
    return { t, windows: out };
  });

  const front = sc.defs[sc.states[0].st.order[0]];
  const scenario = {
    target: { mode: 'display', app: front.app, bundleId: front.bundleId, windowTitle: front.title, displayId: 1, frame: { x: 0, y: 0, width: SW, height: SH, scale: 2 } },
    durationMs: sc.duration, note: sc.description, strokes, events: [...events, ...sc.keys].sort((a, b) => a.t - b.t),
    words: sc.wordList, segments: sc.segments, windows, cursor,
    frames: sc.states.map(({ t }, i) => ({ t, file: `states/${String(i).padStart(2, '0')}.png` })),
  };
  const r = (x) => x.map((v) => Math.round(v));
  const truth = {
    scenario: sc.name, description: sc.description, screen: [SW, SH],
    whole: r(look(sc.wholeSpec, sc.duration / 2)),
    spans: sc.spans.map((s) => {
      const mid = Math.min(s.t1 - 1, s.t0 + Math.max(300, (s.t1 - s.t0) * 0.6));
      return { t0: s.t0, t1: s.t1, subject: s.subject, ideal: r(look(s.ideal, mid)), targets: s.targets.map((x) => r(look(x, mid))) };
    }),
  };
  writeFileSync(join(dir, 'scenario.json'), JSON.stringify(scenario));
  writeFileSync(join(dir, 'truth.json'), JSON.stringify(truth, null, 2));
  return dir;
}

const { chromium } = await loadPlaywright();
const browserApp = await chromium.launch();
const page = await browserApp.newPage({ viewport: { width: SW, height: SH }, deviceScaleFactor: FRAME_W / SW });
await page.setContent(`<!doctype html><html><head><style>${CSS}</style></head><body></body></html>`);
mkdirSync(OUT, { recursive: true });
for (const d of ALL) {
  if (ONLY.size && !ONLY.has(d.name)) continue;
  const sc = new Scenario(d.name, d.description, d.duration);
  d.build(sc);
  console.log(await run(page, sc));
}
await browserApp.close();
