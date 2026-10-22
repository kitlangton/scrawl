#!/usr/bin/env python3
"""Audit an unstripped Linux build's CPU tiers with GNU objdump (#82)."""

import pathlib
import re
import subprocess
import sys


def instructions(path):
    # -d disassembles executable sections only; a byte search also matches data.
    result = subprocess.run(
        ["objdump", "-d", "-C", "--insn-width=16", str(path)],
        check=True, capture_output=True, text=True,
    )
    symbol = ""
    for line in result.stdout.splitlines():
        header = re.match(r"^[0-9a-f]+ <(.+)>:$", line)
        if header:
            symbol = header[1]
        # Binutils prints "{vex} vpdpbusd" to distinguish it from the EVEX
        # encoding. The decoration is not the instruction mnemonic.
        insn = re.match(r"^\s*[0-9a-f]+:\s+((?:[0-9a-f]{2} )+)\s*(?:\{\w+\}\s+)*(\S+)(.*)", line)
        if insn:
            yield symbol, bytes.fromhex(insn[1]), insn[2], line.strip()


def extensions(code, mnemonic):
    # EVEX includes 128/256-bit forms too. Zen 4 has AVX512-VNNI but NOT
    # VEX-encoded AVX-VNNI: checking only zmm operands misses the actual crash.
    code = code.lstrip(b"\x64\x65\x66\x67")
    found = set()
    if code.startswith(b"\x62"):
        found.add("avx512")
    if code[:1] in (b"\xc4", b"\xc5") and mnemonic.startswith("vpdp"):
        found.add("avx-vnni")
    if mnemonic.startswith(("tile", "tdp", "ldtilecfg", "sttilecfg")):
        found.add("amx")
    return found


TIERS = {
    **dict.fromkeys(("x64", "sse42", "sandybridge", "ivybridge", "piledriver", "haswell"), set()),
    "alderlake": {"avx-vnni"},
    **dict.fromkeys(("skylakex", "cannonlake", "cascadelake", "icelake", "cooperlake", "zen4"), {"avx512"}),
    "sapphirerapids": {"avx512", "amx"},
}


def check(directory):
    libraries = {"libtranscribe.so": set(), "libggml.so": set(), "libggml-base.so": set()}
    libraries.update({f"libggml-cpu-{tier}.so": features for tier, features in TIERS.items()})
    needed = subprocess.check_output(["readelf", "-d", str(directory / "scrawl")], text=True)
    if not re.search(r"NEEDED.*\[libtranscribe\.so(?:\.[^]]+)?\]", needed):
        raise RuntimeError("scrawl must dynamically link libtranscribe, not embed native ggml")
    failed = []
    observed = {}
    for name, allowed in libraries.items():
        path = directory / name
        if not path.is_file():
            raise RuntimeError(f"missing {path}")
        seen = set()
        count = 0
        for symbol, code, mnemonic, line in instructions(path):
            count += 1
            features = extensions(code, mnemonic)
            seen |= features
            # The score function runs even when the tier is not selected. ggml
            # builds it without ISA flags and LTO; do not exempt it with its tier.
            permitted = set() if "ggml_backend_score" in symbol or "cpuid_x86" in symbol else allowed
            if features - permitted:
                failed.append(f"{name}: {symbol}: {line}")
        if not count:
            raise RuntimeError(f"objdump decoded no instructions in {path}")
        observed[name] = seen
        print(f"{name}: {count} instructions; {', '.join(sorted(seen)) or 'baseline'}")
    # Positive controls: neither the disassembler nor the detector may silently
    # pass by seeing nothing (or confusing the two encodings of vpdpbusd).
    for tier, feature in [("skylakex", "avx512"), ("alderlake", "avx-vnni"), ("sapphirerapids", "amx")]:
        if feature not in observed[f"libggml-cpu-{tier}.so"]:
            raise RuntimeError(f"no {feature} found in {tier}: broken audit or missing optimization")
    if failed:
        raise RuntimeError("instructions outside their CPU tier:\n" + "\n".join(failed[:20]))


if __name__ == "__main__":
    check(pathlib.Path(sys.argv[1]))
