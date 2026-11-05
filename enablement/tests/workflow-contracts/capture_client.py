"""Guard cold-cache preparation and provenance ordering for native client artifacts."""
from pathlib import Path
import unittest

WORKFLOW = Path(__file__).resolve().parents[3] / ".github/workflows/verification-platform.yml"


class CaptureClientWorkflowTests(unittest.TestCase):
    def test_locked_online_fetch_follows_toolchain_and_cache_before_offline_tests(self):
        text = WORKFLOW.read_text()
        start = text.index("- name: Prepare locked capture-client dependencies for cold cache")
        fetch = text.index("cargo fetch --locked --manifest-path capture/agent/control-client/Cargo.toml", start)
        tests = text.index("cargo test --locked --offline --manifest-path capture/agent/control-client/Cargo.toml", fetch)
        build = text.index("cargo build --locked --offline --manifest-path capture/agent/control-client/Cargo.toml", tests)
        cache = text.index("uses: Swatinem/rust-cache@v2", text.index("  rust:"))
        self.assertLess(cache, start)
        self.assertLess(start, fetch)
        self.assertLess(fetch, tests)
        self.assertLess(tests, build)
        block = text[start:tests]
        self.assertIn("inputs.revision == github.sha", block)
        self.assertIn("--locked", block)
        self.assertNotIn("--offline", block)

    def test_target_and_attestation_remain_bound_to_exact_source(self):
        text = WORKFLOW.read_text()
        producer = text.index("node --experimental-strip-types capture/agent/control-client/delivery/package.ts pack")
        attest = text.index("uses: actions/attest-build-provenance@v2", producer)
        upload = text.index("uses: actions/upload-artifact@v4", attest)
        self.assertLess(producer, attest)
        self.assertLess(attest, upload)
        self.assertIn('"$GITHUB_SHA" "$target"', text[producer:attest])
        self.assertIn("capture-client-${{ inputs.runner }}-${{ github.sha }}", text[upload:])


if __name__ == "__main__":
    unittest.main()
