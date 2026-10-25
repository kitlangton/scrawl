# /// script
# dependencies = ["pillow"]
# ///
"""Renders every Settings and onboarding state twice, with SwiftUI (`scrawl-helper settings|onboarding
--render`) and with Rune (`scrawl app --scene settings-…|onboarding-… --snap`), and compares them.
SwiftUI's windows are the goldens: a state fails when more than --max percent of its pixels differ
by more than 40/255.

    uv run scripts/check-rune-windows.py [--bin target/debug] [--max 2.5] [--sheet DIR] [settings|onboarding]

--sheet writes SwiftUI | Rune | diff images and an index.html per kind.
"""
import argparse, html, os, re, subprocess, sys, tempfile
from PIL import Image, ImageChops

ap = argparse.ArgumentParser()
ap.add_argument("kinds", nargs="*", default=["settings", "onboarding"])
ap.add_argument("--bin", default="target/debug")
ap.add_argument("--max", type=float, default=2.5)
ap.add_argument("--sheet")
args = ap.parse_args()

work = tempfile.mkdtemp(prefix="rune-windows-")
failed = []
for kind in args.kinds:
    swift_dir = os.path.join(work, "swift", kind)
    subprocess.run([os.path.join(args.bin, "scrawl-helper"), kind, "--render", swift_dir], check=True, capture_output=True)
    files = sorted(f for f in os.listdir(swift_dir) if f.endswith(".png"))
    rows = []
    for f in files:
        # 03-general-listening-dark.png → settings-general-listening, …-light.png → …@light
        name = re.sub(r"^\d+-", "", f[:-4])
        name = re.sub(r"-dark$", "", name)
        name = re.sub(r"-light$", "@light", name)
        scene = f"{kind}-{name}"
        rune_png = os.path.join(work, "rune", scene + ".png")
        os.makedirs(os.path.dirname(rune_png), exist_ok=True)
        subprocess.run([os.path.join(args.bin, "scrawl"), "app", "--scene", scene, "--snap", rune_png], capture_output=True)
        s = Image.open(os.path.join(swift_dir, f)).convert("RGBA")
        under = Image.new("RGBA", s.size, (244, 244, 244, 255) if "@light" in name else (30, 30, 30, 255))
        s = Image.alpha_composite(under, s).convert("RGB")
        if not os.path.exists(rune_png):
            failed.append((scene, "no render"))
            continue
        r = Image.open(rune_png).convert("RGB")
        if r.size != s.size:
            failed.append((scene, f"size {r.size} vs SwiftUI {s.size}"))
            continue
        diff = ImageChops.difference(s, r).convert("L").point(lambda v: 255 if v > 40 else 0)
        score = diff.histogram()[255] / (s.size[0] * s.size[1]) * 100
        print(f"{score:5.1f}%  {scene}")
        if score > args.max:
            failed.append((scene, f"{score:.1f}% of pixels differ"))
        if args.sheet:
            w, h = s.size
            out = Image.new("RGB", (w * 3 + 48, h), (40, 40, 40))
            out.paste(s, (0, 0))
            out.paste(r, (w + 24, 0))
            tint = Image.new("RGB", s.size, (255, 40, 40))
            out.paste(Image.composite(tint, s.point(lambda v: v // 3), diff), (2 * (w + 24), 0))
            os.makedirs(os.path.join(args.sheet, kind), exist_ok=True)
            out.save(os.path.join(args.sheet, kind, f))
            rows.append((f, scene, score))
    if args.sheet:
        with open(os.path.join(args.sheet, kind, "index.html"), "w") as page:
            page.write(f"<!doctype html><meta charset=utf-8><title>{kind}: SwiftUI | Rune</title>"
                       "<style>body{background:#111;color:#ddd;font:13px -apple-system;margin:24px}"
                       "img{width:100%;max-width:1900px;display:block;margin:6px 0 28px}</style>")
            for f, scene, score in rows:
                page.write(f"<h2>{html.escape(scene)} ({score:.1f}% differ)</h2><img src='{html.escape(f)}' loading=lazy>")

for scene, why in failed:
    print(f"FAIL {scene}: {why}", file=sys.stderr)
sys.exit(1 if failed else 0)
