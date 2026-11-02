#!/usr/bin/env python3
"""Synthetic installer execution: never builds, contacts SSH or touches a real app."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parent


class InstallerGuardTests(unittest.TestCase):
    def run_installer(self, remote, running=None, ps_status=0, no_launch=False):
        with tempfile.TemporaryDirectory(prefix="installer-guard-") as temporary:
            root = Path(temporary)
            home = root / "Test User"
            repo = home / "code/open-source/apparent"
            packaging = repo / "capture/desktop/scripts/packaging"
            packaging.mkdir(parents=True)
            tools = root / "tools"
            tools.mkdir()
            log = root / "tools.log"
            app = home / "Applications/Apparent.app"
            executable = app / "Contents/MacOS/apparent"
            executable.parent.mkdir(parents=True)
            executable.write_text("old app")
            processes = root / "processes"
            # comm is executable identity, not argv: unrelated CLIs must survive.
            unrelated = [
                "/Applications/Other Apparent.app/Contents/MacOS/apparent",
                str(repo / "target/debug/apparent"),
                str(app) + ".backup/Contents/MacOS/apparent",
                "/usr/bin/ssh",
            ]
            entries = unrelated + ([str(app / running)] if running else [])
            processes.write_text("".join(f"  {100 + i} {path}\n" for i, path in enumerate(entries)))
            mock = '''#!/usr/bin/env bash
set -eu
name="${0##*/}"
printf '%s\\n' "$name $*" >> "$MOCK_LOG"
case "$name" in
  ps) cat "$MOCK_PROCESSES"; exit "$MOCK_PS_STATUS" ;;
  ssh) shift; exec "$@" ;;
  launchctl)
    if [[ "$1" == bootstrap ]]; then
      for file in "$MOCK_TMP"/*.plist; do
        bash -n "${file%.plist}.sh"
        printf 'STAGE-EXIT=0\\n' > "${file%.plist}.log"
      done
    fi ;;
  security) exit 1 ;;
  taskpolicy|cargo|git|codesign|open|sleep|pgrep|pkill|ditto) : ;;
  *) exit 90 ;;
esac
'''
            for name in ("ps", "ssh", "launchctl", "security", "taskpolicy", "cargo", "git",
                         "codesign", "open", "sleep", "pgrep", "pkill", "ditto"):
                tool = tools / name
                tool.write_text(mock)
                tool.chmod(0o755)
            for relative in ("target/debug/apparent", "target/debug/apparent-helper",
                             "capture/desktop/assets/Apparent.icns",
                             "shared/assets/fonts/HostGrotesk-Variable.ttf",
                             "shared/assets/brand/apparent-wordmark.png",
                             "shared/assets/brand/apparent-a.png"):
                asset = repo / relative
                asset.parent.mkdir(parents=True, exist_ok=True)
                asset.write_text("synthetic new asset")
            release = repo / "capture/desktop/scripts/release"
            release.mkdir()
            for name in ("setup-sparkle.sh", "release-app.sh"):
                (release / name).write_text("#!/usr/bin/env bash\nexit 0\n")
            for name in ("install-app.sh", "install-app-remote.sh"):
                source = (SCRIPTS / name).read_text()
                # Redirect fixed native paths into this disposable synthetic fixture.
                source = source.replace("/usr/bin/ditto", str(tools / "ditto"))
                source = source.replace("/tmp/", str(root) + "/")
                (packaging / name).write_text(source)
            env = dict(os.environ, HOME=str(home), PATH=str(tools) + ":" + os.environ["PATH"],
                       MOCK_LOG=str(log), MOCK_PROCESSES=str(processes),
                       MOCK_PS_STATUS=str(ps_status), MOCK_TMP=str(root),
                       APPARENT_APP_DIR=str(app), APPARENT_NO_LAUNCH="1" if no_launch else "0")
            script = packaging / ("install-app-remote.sh" if remote else "install-app.sh")
            result = subprocess.run(["bash", str(script)], env=env, text=True,
                                    capture_output=True, timeout=10)
            calls = log.read_text() if log.exists() else ""
            self.assertNotIn("pkill ", calls, result.stderr)
            refused = running is not None or ps_status != 0
            if refused:
                self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                self.assertEqual(executable.read_text(), "old app")
                self.assertFalse((app / "Contents/Info.plist").exists())
                self.assertNotIn("ditto ", calls)
                self.assertNotIn("open ", calls)
                self.assertIn("retry", result.stderr)
                if running:
                    self.assertIn("finish", result.stderr)
                    self.assertIn("quit", result.stderr)
            else:
                self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
                if remote:
                    self.assertIn("ditto ", calls)
                else:
                    self.assertEqual(executable.read_text(), "synthetic new asset")
                self.assertEqual("open " in calls, remote or not no_launch)

    def test_destination_processes_refuse_before_replacement(self):
        for remote in (False, True):
            for running in ("Contents/MacOS/apparent", "Contents/MacOS/apparent-helper",
                            "Contents/Frameworks/Sparkle.framework/Autoupdate"):
                for no_launch in (False, True):
                    with self.subTest(remote=remote, running=running, no_launch=no_launch):
                        self.run_installer(remote, running, no_launch=no_launch)

    def test_process_listing_failure_refuses(self):
        for remote in (False, True):
            with self.subTest(remote=remote):
                self.run_installer(remote, ps_status=2)

    def test_unrelated_apps_and_clis_do_not_block(self):
        for remote in (False, True):
            for no_launch in (False, True):
                with self.subTest(remote=remote, no_launch=no_launch):
                    self.run_installer(remote, no_launch=no_launch)


if __name__ == "__main__":
    unittest.main(verbosity=2)
