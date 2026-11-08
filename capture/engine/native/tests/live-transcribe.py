#!/usr/bin/env python3
"""Native source/converter/completion regression tests; no audio devices or models."""
from pathlib import Path
import platform
import subprocess
import tempfile

root = Path(__file__).resolve().parents[4]
if platform.system() != "Darwin":
    raise SystemExit("These tests require macOS, AVFoundation and Swift 6.2+")
with tempfile.TemporaryDirectory(prefix="apparent-live-tests-") as directory:
    scratch = Path(directory)
    combined = scratch / "LiveTests.swift"
    combined.write_bytes((root / "capture/engine/native/LiveTranscribe.swift").read_bytes()
                         + b"\n" + Path(__file__).with_suffix(".swift").read_bytes())
    executable = scratch / "live-tests"
    subprocess.run(["xcrun", "swiftc", "-O", "-swift-version", "5", "-parse-as-library",
                    str(root / "capture/engine/native/Media.swift"), str(combined),
                    "-Xlinker", "-sectcreate", "-Xlinker", "__TEXT", "-Xlinker", "__info_plist",
                    "-Xlinker", str(root / "shared/native/Info.plist"), "-o", str(executable)],
                   check=True, timeout=120)
    subprocess.run([str(executable)], check=True, timeout=30)
