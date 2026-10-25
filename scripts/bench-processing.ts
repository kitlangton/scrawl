import { mkdtemp, readdir, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

// Times bundle processing: one warmup and seven `scrawl demo` runs (synthetic frames, no recording,
// transcription or clipboard), plus the per-stage `BENCH` timings `SCRAWL_BENCH=1` prints.
//   CARGO_TARGET_DIR=<target> nice -n 10 bun scripts/bench-processing.ts
// Build separately: timing must never include cargo or depend on its cache state. `SCRAWL_BIN`
// picks a saved control binary to compare against.
const binary = resolve(process.env.SCRAWL_BIN ?? join(process.env.CARGO_TARGET_DIR ?? "target", "debug/scrawl"));
const root = await mkdtemp(join(tmpdir(), "scrawl-processing-"));
const samples: number[] = [];
const stages: Record<string, number[]> = {};
let pngBytes = 0;
const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)]!;
const summarize = (xs: number[]) => {
  const mid = median(xs);
  return { median: mid, mad: median(xs.map(x => Math.abs(x - mid))), min: Math.min(...xs), max: Math.max(...xs) };
};
try {
  for (let run = 0; run < 8; run++) {
    const out = join(root, `${run}.scrawl`);
    const start = performance.now();
    const child = Bun.spawn([binary, "demo", "--out", out], {
      env: { ...process.env, SCRAWL_BENCH: "1" }, stdout: "pipe", stderr: "pipe",
    });
    const [code, stdout, stderr] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
    const ms = performance.now() - start;
    if (code !== 0) throw new Error(`demo exited ${code}\n${stderr}\n${stdout}`);
    console.log(`${run === 0 ? "warmup" : `run ${run}`}: ${ms.toFixed(3)} ms`);
    if (run > 0) {
      samples.push(ms);
      for (const match of stderr.matchAll(/BENCH (\w+)=([\d.]+)/g)) {
        (stages[match[1]!] ??= []).push(Number(match[2]));
      }
    }
    if (run === 7) {
      for (const path of [join(out, "contact-sheet.png"), ...(await readdir(join(out, "frames"))).map(name => join(out, "frames", name))]) {
        pngBytes += (await stat(path)).size;
      }
    }
  }
  const stats = summarize(samples);
  console.log(JSON.stringify({ binary, samples, ...stats, stages: Object.fromEntries(Object.entries(stages).map(([key, xs]) => [key, summarize(xs)])), pngBytes }));
  console.log(`METRIC bundle_processing_ms=${stats.median.toFixed(3)}`);
} finally {
  await rm(root, { recursive: true, force: true });
}
