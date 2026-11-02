"""Opt-in real CLI/encoder/Kokoro proof, shared by macOS and Linux.

APPARENT_EXPORT_TEST_BINARY=/absolute/apparent python3 -m unittest discover \
    -s capture/desktop/scripts/tests -p test_export_audio.py -v
Provision the default narrator first. Optional APPARENT_EXPORT_TEST_OUTPUT retains
private synthetic evidence in a new directory. Never uses a real capture or speaker.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil

import subprocess
import tempfile
import unittest


@unittest.skipUnless(os.environ.get("APPARENT_EXPORT_TEST_BINARY"), "explicit built binary required")
class ExportAudioProof(unittest.TestCase):
    def test_real_audio_policy_and_narration(self):
        binary = str(Path(os.environ["APPARENT_EXPORT_TEST_BINARY"]).resolve())
        retained = os.environ.get("APPARENT_EXPORT_TEST_OUTPUT")
        temporary = tempfile.TemporaryDirectory(prefix="apparent-export-proof-") if not retained else None
        root = Path(temporary.name if temporary else str(retained)).resolve()
        if retained:
            root.mkdir(mode=0o700, parents=True, exist_ok=False)
        logs = []
        env = dict(os.environ, APPARENT_CONFIG=str(root / "config.json"))
        (root / "config.json").write_text(json.dumps({}))

        def run(args, success=True):
            result = subprocess.run(list(map(str, args)), env=env, capture_output=True, timeout=180)
            logs.append({"args": list(map(str, args)), "exit": result.returncode,
                         "stdout": "<PCM decoded in memory>" if "pipe:1" in args else result.stdout.decode(errors="replace"),
                         "stderr": result.stderr.decode(errors="replace")})
            (root / "commands.json").write_text(json.dumps(logs, indent=2))
            self.assertEqual(result.returncode == 0, success, logs[-1])
            return result

        def probe(path):
            result = run(["ffprobe", "-v", "error", "-show_streams", "-show_format", "-of", "json", path])
            return json.loads(result.stdout)

        def export(name, *flags, success=True):
            return run([binary, "export", bundle, "--out", root / name,
                        "--size", "320", "--fps", "10", "--no-zoom", *flags], success)

        def snapshot():
            return {str(p.relative_to(bundle)): hashlib.sha256(p.read_bytes()).hexdigest()
                    for p in bundle.rglob("*") if p.is_file()}

        def audio(path):
            data = run(["ffmpeg", "-v", "error", "-i", path, "-map", "0:a:0",
                        "-f", "s16le", "-ac", "1", "-ar", "24000", "pipe:1"]).stdout
            # Avoid writing decoded audio or binary stdout into the command log.
            return [int.from_bytes(data[i:i+2], "little", signed=True) for i in range(0, len(data), 2)]

        try:
            bundle = root / "synthetic.apparent"
            run([binary, "demo", "--out", bundle])
            manifest = bundle / "apparent.json"
            data = json.loads(manifest.read_text())
            data.update(video="recording.mp4", audio="audio.wav", durationMs=12000,
                        videoOffsetMs=0, events=[], strokes=[], cursor=[])
            manifest.write_text(json.dumps(data))
            run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i",
                 "testsrc2=size=320x180:rate=10:duration=12", "-c:v", "libx264",
                 "-pix_fmt", "yuv420p", bundle / "recording.mp4"])
            run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i",
                 "sine=frequency=440:duration=12", "-ar", "16000", "-ac", "1", bundle / "audio.wav"])
            original = snapshot()
            export("recorded.mp4")
            self.assertTrue(any(s["codec_type"] == "audio" for s in probe(root / "recorded.mp4")["streams"]))
            export("silent.mp4", "--no-audio")
            self.assertFalse(any(s["codec_type"] == "audio" for s in probe(root / "silent.mp4")["streams"]))
            self.assertEqual(snapshot(), original)

            # Export publication must not replace the narration input, including
            # aliases reached through a symlinked parent directory.
            alias = root / "script-alias"
            alias.symlink_to(root, target_is_directory=True)
            protected_script = root / "protected-script.json"
            script_bytes = b'{"version":1,"cues":[{"startMs":500,"endMs":5000,"text":"Keep this script unchanged."}]}'
            for output in [protected_script, alias / protected_script.name]:
                protected_script.write_bytes(script_bytes)
                failure = export(output, "--narration", protected_script, success=False)
                self.assertNotEqual(failure.returncode, 0)
                self.assertIn("cannot replace narration script", failure.stderr.decode())
                self.assertEqual(protected_script.read_bytes(), script_bytes)
            alias.unlink()

            # Admission and mux must select the same overlapping native iPhone track.
            phone = root / "phone.apparent"
            shutil.copytree(bundle, phone)
            (phone / "recording.mp4").unlink()
            phone_data = dict(data, source="iphone")
            (phone / "apparent.json").write_text(json.dumps(phone_data))
            run(["ffmpeg", "-v", "error", "-i", bundle / "recording.mp4",
                 "-itsoffset", "20", "-f", "lavfi", "-i", "sine=frequency=220:duration=1",
                 "-f", "lavfi", "-i", "sine=frequency=880:duration=12",
                 "-map", "0:v:0", "-map", "1:a:0", "-map", "2:a:0", "-c:v", "copy",
                 "-c:a", "aac", phone / "recording.mp4"])
            run([binary, "export", phone, "--out", root / "phone.mp4", "--size", "320", "--fps", "10", "--no-zoom"])
            native_samples = audio(root / "phone.mp4")[24000:48000]
            self.assertGreater(max(map(abs, native_samples)), 1000)
            crossings = sum(a <= 0 < b for a, b in zip(native_samples, native_samples[1:]))
            self.assertAlmostEqual(crossings, 880, delta=20, msg="must prefer native 880 Hz over WAV 440 Hz")
            run(["ffmpeg", "-v", "error", "-i", phone / "recording.mp4",
                 "-map", "0:v:0", "-map", "0:a:0", "-c", "copy", phone / "disjoint.mp4"])
            phone_data.update(video="disjoint.mp4", audio=None)
            (phone / "apparent.json").write_text(json.dumps(phone_data))
            disjoint = run([binary, "export", phone, "--out", root / "disjoint-new/out.mp4"], success=False)
            self.assertIn("[AUDIO_MISSING]", disjoint.stderr.decode())
            self.assertFalse((root / "disjoint-new").exists())

            saved_wav = (bundle / "audio.wav").read_bytes()
            (bundle / "audio.wav").unlink()
            missing = snapshot()
            result = export("not-created/missing.mp4", success=False)
            message = result.stderr.decode()
            for fragment in ["[AUDIO_MISSING]", str(bundle), "--narration", "--no-audio"]:
                self.assertIn(fragment, message)
            self.assertFalse((root / "not-created").exists())
            script = root / "cues.json"
            script.write_text(json.dumps({"version": 1, "cues": [
                {"startMs": 500, "endMs": 5000, "text": "This is local export narration."},
                {"startMs": 6500, "endMs": 11000, "text": "The original capture stays unchanged."}]}))
            export("conflict.mp4", "--no-audio", "--narration", script, success=False)
            self.assertFalse((root / "conflict.mp4").exists())
            bad = root / "bad.json"
            bad.write_text('{"version":1,"cues":[{"startMs":0}]}')
            self.assertIn("[NARRATION_SCRIPT]", export("invalid/out.mp4", "--narration", bad,
                                                      success=False).stderr.decode())
            self.assertFalse((root / "invalid").exists())
            (root / "config.json").write_text('{"experiments":{"exportNarration":false}}')
            self.assertIn("[NARRATION_DISABLED]", export("disabled/out.mp4", "--narration", script,
                                                        success=False).stderr.decode())
            self.assertFalse((root / "disabled").exists())
            (root / "config.json").write_text('{}')
            export("narrated.mp4", "--narration", script)
            metadata = probe(root / "narrated.mp4")
            self.assertEqual([s["codec_name"] for s in metadata["streams"]], ["h264", "aac"])
            self.assertAlmostEqual(float(metadata["format"]["duration"]), 12, delta=0.15)
            run(["ffmpeg", "-v", "error", "-i", root / "narrated.mp4", "-f", "null", "-"])
            pcm = audio(root / "narrated.mp4")
            def energy(start, end):
                segment = pcm[int(start * 24000):int(end * 24000)]
                return sum(x * x for x in segment) / max(1, len(segment))
            self.assertGreater(energy(0.7, 2.5), 10000)
            self.assertGreater(energy(6.8, 8.5), 10000)
            self.assertLess(energy(0, 0.35), energy(0.7, 2.5) / 100)
            self.assertLess(energy(5.3, 6.2), energy(6.8, 8.5) / 100)
            self.assertEqual(snapshot(), missing)
            (bundle / "audio.wav").write_bytes(saved_wav)
            self.assertEqual(snapshot(), original)
            (root / "result.json").write_text(json.dumps({"binary": binary,
                "binarySha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
                "sourceHashes": original, "streams": metadata,
                "result": "real recorded/silent/missing/narrated policies passed"}, indent=2))
        finally:
            if temporary:
                temporary.cleanup()


if __name__ == "__main__":
    unittest.main()
