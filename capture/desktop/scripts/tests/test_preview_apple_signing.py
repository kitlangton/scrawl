"""Executable release-boundary tests; fake Mac tools do not prove Apple acceptance."""
import base64
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap
import unittest

ROOT = Path(__file__).resolve().parents[4]


class AppleSigningTest(unittest.TestCase):
    def fixture(self, root):
        shutil.copytree(ROOT / "capture/desktop/scripts", root / "capture/desktop/scripts")
        shutil.copytree(ROOT / "capture/desktop/assets", root / "capture/desktop/assets")
        shutil.copytree(ROOT / "shared/assets", root / "shared/assets")
        (root / "bin").mkdir()
        (root / "sparkle/Sparkle.framework/Versions/B/XPCServices").mkdir(parents=True)
        setup = root / "capture/desktop/scripts/release/setup-sparkle.sh"
        setup.write_text('#!/bin/sh\nprintf "%s/sparkle\\n" "$PWD"\n')
        setup.chmod(0o755)
        tools = {
            "cargo": 'mkdir -p target/release; touch target/release/apparent target/release/apparent-helper',
            "plutil": 'exit 0',
            "codesign": '''printf 'codesign %s\\n' "$*" >> "$TRACE"
if [ "$1" = -dv ]; then
  echo 'Authority=Developer ID Application: Test (TESTTEAM01)' >&2
  echo "TeamIdentifier=${ACTUAL_TEAM:-TESTTEAM01}" >&2
fi''',
            "hdiutil": '''printf 'hdiutil %s\\n' "$*" >> "$TRACE"
if [ "$1" = create ]; then eval "out=\\${$#}"; printf dmg > "$out"; fi''',
            "ditto": '''printf 'ditto %s\\n' "$*" >> "$TRACE"
if [ "$1" = -c ]; then eval "out=\\${$#}"; printf zip > "$out"; else cp -R "$1" "$2"; fi''',
            "xcrun": '''printf 'xcrun %s\\n' "$*" >> "$TRACE"
if [ "$1" = notarytool ] && [ "${REJECT_NOTARY:-0}" = 1 ]; then exit 1; fi''',
            "spctl": 'printf "spctl %s\\n" "$*" >> "$TRACE"',
        }
        for name, body in tools.items():
            tool = root / "bin" / name
            tool.write_text('#!/bin/sh\nset -eu\n' + body + '\n')
            tool.chmod(0o755)
        signing = root / "signing"
        signing.mkdir()
        (signing / "notary.p8").write_text("synthetic test key")
        (signing / "signing.keychain-db").touch()
        return dict(os.environ, PATH=f'{root / "bin"}:{os.environ["PATH"]}',
                    TRACE=str(root / "trace"), APPARENT_PREVIEW_VERSION="0.123.2",
                    APPARENT_PREVIEW_BUILD_NUMBER="123.2",
                    APPARENT_PREVIEW_PUBLIC_KEY=base64.b64encode(bytes([1]) * 32).decode(),
                    APPARENT_PREVIEW_NOTARIZE="true", APPARENT_APPLE_SIGNING_DIR=str(signing),
                    APPARENT_APPLE_SIGNING_IDENTITY="Developer ID Application: Test (TESTTEAM01)",
                    APPARENT_APPLE_TEAM_ID="TESTTEAM01", APPARENT_NOTARY_KEY_ID="TESTKEY001",
                    APPARENT_NOTARY_ISSUER="00000000-0000-0000-0000-000000000001")

    def run_build(self, root, env):
        return subprocess.run(["bash", str(root / "capture/desktop/scripts/packaging/build-mac-preview.sh")],
                              cwd=root, env=env, text=True, capture_output=True)

    def test_requested_notarization_never_falls_back_to_adhoc(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = self.fixture(root)
            del env["APPARENT_APPLE_SIGNING_IDENTITY"]
            result = self.run_build(root, env)
            self.assertNotEqual(result.returncode, 0, "unsigned release accepted despite notarization request")
            self.assertFalse((root / "dist").exists())

    def test_signed_packages_are_verified_and_stapled_before_archiving(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = self.fixture(root)
            result = self.run_build(root, env)
            self.assertEqual(result.returncode, 0, result.stderr)
            trace = (root / "trace").read_text()
            self.assertIn("--options runtime --timestamp", trace)
            self.assertNotIn("--sign - ", trace)
            app_staple = trace.index("stapler staple " + str(root / "dist/Apparent Preview.app"))
            self.assertLess(app_staple, trace.index("hdiutil create"))
            self.assertIn("stapler validate", trace)
            self.assertIn("spctl --assess", trace)
            self.assertLess(trace.index("stapler staple " + str(root / "dist/Apparent-Preview")),
                            trace.rindex("ditto -c"))

    def test_rejection_or_wrong_team_prevents_update_zip(self):
        for extra in ({"REJECT_NOTARY": "1"}, {"ACTUAL_TEAM": "WRONGTEAM1"}):
            with self.subTest(extra=extra), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                env = self.fixture(root)
                env.update(extra)
                result = self.run_build(root, env)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse((root / "dist/Apparent-Preview-0.123.2-arm64.zip").exists())

    def test_unknown_signing_mode_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = self.fixture(root)
            env["APPARENT_PREVIEW_NOTARIZE"] = "yes"
            self.assertNotEqual(self.run_build(root, env).returncode, 0)

    def test_private_keychain_setup_and_failed_import_cleanup(self):
        for fail in (False, True):
            with self.subTest(fail=fail), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / "bin").mkdir()
                security = root / "bin/security"
                security.write_text('''#!/bin/sh
set -eu
printf '%s\\n' "$1" >> "$TRACE"
case "$1" in
  create-keychain) eval "out=\\${$#}"; touch "$out" ;;
  delete-keychain) rm -f "$2" ;;
  import) if [ "$FAIL_IMPORT" = 1 ]; then exit 1; fi ;;
  find-identity) echo '1) HASH "Developer ID Application: Test (TESTTEAM01)"' ;;
  list-keychains)
    if [ "$#" = 3 ]; then
      printf '    "/Users/runner/Library/Keychains/login.keychain-db"\\n    "/Users/runner/Library/Keychains/existing custom.keychain-db"\\n'
    else
      shift 4
      python3 -c 'import json, os, sys; from pathlib import Path; Path(os.environ["SEARCH_LIST"]).write_text(json.dumps(sys.argv[1:]))' "$@"
    fi ;;
esac
''')
                security.chmod(0o755)
                env = dict(os.environ, PATH=f'{root / "bin"}:{os.environ["PATH"]}',
                           RUNNER_TEMP=tmp, TRACE=str(root / "trace"), FAIL_IMPORT=str(int(fail)),
                           SEARCH_LIST=str(root / "search-list.json"),
                           APPARENT_APPLE_SIGNING_IDENTITY="Developer ID Application: Test (TESTTEAM01)",
                           APPARENT_APPLE_TEAM_ID="TESTTEAM01", APPARENT_NOTARY_KEY_ID="TESTKEY001",
                           APPARENT_NOTARY_ISSUER="00000000-0000-0000-0000-000000000001",
                           APPLE_DEVELOPER_ID_P12_BASE64=base64.b64encode(b'synthetic certificate').decode(),
                           APPLE_DEVELOPER_ID_P12_PASSWORD="synthetic-password",
                           APPLE_NOTARY_PRIVATE_KEY="-----BEGIN PRIVATE KEY-----\nsynthetic\n-----END PRIVATE KEY-----")
                result = subprocess.run(["bash", str(ROOT / "capture/desktop/scripts/release/prepare-mac-signing.sh")],
                                        env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode == 0, not fail, result.stderr)
                directory = root / "apparent-apple-signing"
                self.assertFalse((directory / "certificate.p12").exists())
                self.assertNotIn("synthetic-password", result.stdout + result.stderr)
                if fail:
                    self.assertFalse(directory.exists())
                    self.assertTrue((root / "trace").exists(), "import/cleanup was never attempted")
                    self.assertIn("delete-keychain", (root / "trace").read_text())
                else:
                    self.assertEqual(directory.stat().st_mode & 0o777, 0o700)
                    self.assertEqual((directory / "notary.p8").stat().st_mode & 0o777, 0o600)
                    self.assertIn("set-key-partition-list", (root / "trace").read_text())
                    self.assertTrue((root / "search-list.json").exists(),
                                    "codesign cannot resolve a name from a keychain outside its search list")
                    self.assertEqual(json.loads((root / "search-list.json").read_text()), [
                        str(directory / "signing.keychain-db"),
                        "/Users/runner/Library/Keychains/login.keychain-db",
                        "/Users/runner/Library/Keychains/existing custom.keychain-db",
                    ])

    def test_workflow_cleanup_removes_key_file_even_when_keychain_delete_fails(self):
        workflow = (ROOT / ".github/workflows/release-mac-preview.yml").read_text()
        block = workflow.split("      - name: Remove Apple signing credentials\n", 1)[1]
        self.assertIn("always()", block)
        script = textwrap.dedent(block.split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            directory = root / "apparent-apple-signing"
            directory.mkdir()
            (directory / "notary.p8").write_text("synthetic")
            (directory / "signing.keychain-db").touch()
            (root / "bin").mkdir()
            security = root / "bin/security"
            security.write_text('#!/bin/sh\nexit 1\n')
            security.chmod(0o755)
            result = subprocess.run(["bash", "-c", script], capture_output=True, text=True,
                                    env=dict(os.environ, RUNNER_TEMP=tmp,
                                             PATH=f'{root / "bin"}:{os.environ["PATH"]}'))
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(directory.exists())

    def test_missing_secret_fails_before_import_without_echoing_other_secrets(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = dict(os.environ, RUNNER_TEMP=tmp,
                       APPLE_DEVELOPER_ID_P12_BASE64="", APPLE_DEVELOPER_ID_P12_PASSWORD="do-not-print-me")
            result = subprocess.run(["bash", str(ROOT / "capture/desktop/scripts/release/prepare-mac-signing.sh")],
                                    env=env, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Missing Apple signing input: APPLE_DEVELOPER_ID_P12_BASE64", result.stderr)
            self.assertNotIn("do-not-print-me", result.stdout + result.stderr)
            self.assertFalse((Path(tmp) / "apparent-apple-signing").exists())


if __name__ == "__main__":
    unittest.main()
