#!/usr/bin/env python3
"""Device-free native writer and transcriber regressions; no capture, audio devices or models."""
from pathlib import Path
import platform
import argparse
import subprocess
import tempfile

root = next(parent for parent in Path(__file__).resolve().parents
            if (parent / 'Cargo.toml').is_file() and (parent / 'engine/native').is_dir())
if platform.system() != "Darwin":
    raise SystemExit("These tests require macOS, AVFoundation and Swift 6.2+")
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--retain-segmented-mp4', type=Path,
                    help='retain validated synthetic writer output in an existing private 0700 directory')
args = parser.parse_args()
with tempfile.TemporaryDirectory(prefix="apparent-live-tests-") as directory:
    scratch = Path(directory)
    combined = scratch / "LiveTests.swift"
    combined.write_bytes((root / "engine/native/LiveTranscribe.swift").read_bytes()
                         + b"\n" + Path(__file__).with_suffix(".swift").read_bytes())
    executable = scratch / "live-tests"
    subprocess.run(["xcrun", "swiftc", "-O", "-swift-version", "5", "-parse-as-library",
                    str(root / "engine/native/Media.swift"), str(combined),
                    "-Xlinker", "-sectcreate", "-Xlinker", "__TEXT", "-Xlinker", "__info_plist",
                    "-Xlinker", str(root / "shared/native/Info.plist"), "-o", str(executable)],
                   check=True, timeout=120)
    subprocess.run([str(executable)], check=True, timeout=30)

    writer_combined = scratch / "SegmentedWriterTests.swift"
    writer_combined.write_bytes((root / "engine/native/Writer.swift").read_bytes()
                                + b"\n" + (root / "engine/native/tests/segmented-writer.swift").read_bytes())
    writer_executable = scratch / "segmented-writer-tests"
    subprocess.run(["xcrun", "swiftc", "-O", "-swift-version", "5", "-parse-as-library",
                    str(writer_combined), "-o", str(writer_executable)],
                   check=True, timeout=120)
    output_directory = args.retain_segmented_mp4.resolve() if args.retain_segmented_mp4 else scratch
    writer_args = [str(writer_executable), str(output_directory)]
    if args.retain_segmented_mp4:
        writer_args.append('--retain')
    subprocess.run(writer_args, check=True, timeout=60)
