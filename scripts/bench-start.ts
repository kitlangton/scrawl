import { mkdtemp, readdir, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

// Times recording start (`recording_start_ms`, main-thread time in `start_recording_with_target`)
// and its stages, through the real picker, Metal overlay, microphone and ScreenCaptureKit.
//   bun scripts/bench-start.ts [--quick-stop]
// `--quick-stop` stops both workers at once, checking clean joins rather than timing. Needs Screen
// Recording and Microphone grants, `ffprobe`, and an awake screen someone is using:
// `perf/screen-guard.sh` refuses on the Studio, so run it on the MacBook Pro under `/tmp/pro-gui.lock`.
// One process: warmup + seven starts, matching repeated captures in the menu-bar app.
// Build separately so compiler/cache state never enters the timing.
const binary = resolve(process.env.SCRAWL_BIN ?? "target/debug/scrawl");
const quickStop = process.argv.includes("--quick-stop");
const root = await mkdtemp(join(tmpdir(), "scrawl-start-"));
const median = (xs: number[]) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)]!;
const summarize = (xs: number[]) => {
  const mid = median(xs);
  return { median: mid, mad: median(xs.map(x => Math.abs(x - mid))), min: Math.min(...xs), max: Math.max(...xs) };
};
const guard = Bun.spawnSync([resolve(import.meta.dir, "../perf/screen-guard.sh")], { stderr: "inherit" });
if (guard.exitCode !== 0) process.exit(1);
try {
  // -i keeps the Mac from idle-sleeping mid-run; never -d/-u, which wake the display.
  const child = Bun.spawn(["caffeinate", "-i", binary], {
    env: { ...process.env, SCRAWL_BENCH_START: root, SCRAWL_BENCH_QUICK_STOP: quickStop ? "1" : "0" }, stdout: "pipe", stderr: "pipe",
  });
  const timeout = setTimeout(() => child.kill(), 120_000);
  const [code, stdout, stderr] = await Promise.all([child.exited, new Response(child.stdout).text(), new Response(child.stderr).text()]);
  clearTimeout(timeout);
  if (code !== 0) throw new Error(`benchmark exited ${code}\n${stderr}\n${stdout}`);
  const runs = stdout.split("\n").filter(line => line.startsWith("START_RUN ")).map(line => JSON.parse(line.slice(10)));
  if (runs.length !== 8) throw new Error(`Expected 8 runs, got ${runs.length}\n${stderr}\n${stdout}`);
  const stages: Record<string, number[]> = {};
  for (const match of stderr.matchAll(/START_METRIC (\w+)=([\d.]+)/g)) {
    (stages[match[1]!] ??= []).push(Number(match[2]));
  }
  for (const key of ["picker_prep_ms", "picker_open_ms", "target_discovery_ms", "metal_open_ms", "audio_start_ms", "sck_start_ms", "recording_start_ms"]) {
    if (stages[key]?.length !== 8) throw new Error(`Missing stage ${key}\n${stderr}`);
  }
  // Decode actual MP4s rather than accepting the capture path's synthetic fallback.
  for (const folder of quickStop ? [] : await readdir(root)) {
    const path = join(root, folder);
    const wav = await readFile(join(path, "audio.wav"));
    if (wav.toString("ascii", 0, 4) !== "RIFF" || wav.length <= 44) throw new Error("Invalid/empty WAV");
    const probe = Bun.spawn(["ffprobe", "-v", "error", "-select_streams", "v:0", "-count_frames", "-show_entries", "stream=nb_read_frames", "-of", "json", join(path, "recording.mp4")], { stdout: "pipe", stderr: "pipe" });
    const [status, output, errors] = await Promise.all([probe.exited, new Response(probe.stdout).text(), new Response(probe.stderr).text()]);
    if (status !== 0 || !(Number(JSON.parse(output).streams?.[0]?.nb_read_frames) > 0)) throw new Error(`Invalid MP4: ${errors} ${output}`);
  }
  const summaries = Object.fromEntries(Object.entries(stages).map(([name, values]) => [name, summarize(values.slice(1))]));
  console.log(JSON.stringify({ binary, quickStop, warmup: runs[0], runs: runs.slice(1), stages: summaries, samples: stages }));
  for (const [name, stats] of Object.entries(summaries)) {
    console.log(`${name}: median=${stats.median.toFixed(3)} MAD=${stats.mad.toFixed(3)} min=${stats.min.toFixed(3)} max=${stats.max.toFixed(3)}`);
    console.log(`METRIC ${name}=${stats.median.toFixed(3)}`);
  }
} finally {
  await rm(root, { recursive: true, force: true });
}
