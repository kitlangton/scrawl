"""Evaluate only owned scripts' root-discovery assignments at their real paths."""

from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[5]
SCRIPTS = (
    "capture/desktop/scripts/packaging/install-app.sh",
    "capture/desktop/scripts/packaging/build-mac-preview.sh",
    "capture/desktop/scripts/release/release-app.sh",
    "capture/desktop/scripts/release/setup-sparkle.sh",
    "capture/desktop/scripts/linux/appimage/appimage.sh",
    "capture/desktop/scripts/linux/appimage/smoke-cpu.sh",
    "capture/desktop/scripts/linux/maintenance/build-playback.sh",
    "capture/desktop/scripts/support/check-ca-roots.sh",
)


SOURCES = {
    "capture/desktop/scripts/packaging/install-app.sh": (
        "capture/desktop/scripts/release/setup-sparkle.sh",
        "capture/desktop/assets/Apparent.icns",
        "shared/assets/fonts/HostGrotesk-Variable.ttf",
    ),
    "capture/desktop/scripts/packaging/build-mac-preview.sh": (
        "capture/desktop/scripts/release/setup-sparkle.sh",
        "capture/desktop/scripts/packaging/Apparent.entitlements",
        "capture/desktop/assets/Apparent.icns",
    ),
    "capture/desktop/scripts/release/release-app.sh": (
        "capture/desktop/scripts/release/setup-sparkle.sh",
        "capture/desktop/scripts/packaging/Apparent.entitlements",
        "shared/assets/fonts/HostGrotesk-Variable.ttf",
    ),
    "capture/desktop/scripts/release/setup-sparkle.sh": ("Cargo.toml",),
    "capture/desktop/scripts/linux/appimage/appimage.sh": (
        "capture/desktop/scripts/support/apparent-restore-clipboard.py",
        "capture/desktop/packaging/linux/dev.kitlangton.apparent.desktop",
        "capture/desktop/scripts/linux/appimage/check-appimage-encoder.sh",
    ),
    "capture/desktop/scripts/linux/appimage/smoke-cpu.sh": ("Cargo.toml",),
    "capture/desktop/scripts/linux/maintenance/build-playback.sh": (
        "capture/desktop/linux/playback/plugin.c",
        "capture/desktop/linux/playback/helper.c",
    ),
    "capture/desktop/scripts/support/check-ca-roots.sh": ("Cargo.lock",),
}


class ScriptRootResolutionTest(unittest.TestCase):
    def test_root_discovery_resolves_repository_without_running_script(self):
        for relative in SCRIPTS:
            with self.subTest(script=relative):
                script = ROOT / relative
                lines = script.read_text().splitlines()
                assignment = next(
                    (line.strip() for line in lines
                     if "dirname" in line and "pwd" in line and "=" in line
                     and line.lstrip().split("=", 1)[0] in {"root", "ROOT", "repo", "REPO_DIR"}),
                    None,
                )
                if assignment is None:
                    self.fail(f"no root-discovery assignment in {relative}")
                variable = assignment.split("=", 1)[0]
                # bash -c has no script file in BASH_SOURCE[0], so model the
                # script-entry value with $0 while evaluating the same dirname/cd path.
                assignment = assignment.replace('${BASH_SOURCE[0]}', '$0')
                # The script itself is never sourced or run (install/release and
                # destructive smoke stay inert).
                result = subprocess.run(
                    ["bash", "-c", f'{assignment}\nprintf \'%s\\n\' "${{{variable}}}"', str(script)],
                    cwd=ROOT,
                    capture_output=True,
                    text=True,
                    check=False,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(Path(result.stdout.strip()), ROOT, relative)
                self.assertTrue((Path(result.stdout.strip()) / "Cargo.toml").is_file())
                for source in SOURCES[relative]:
                    self.assertTrue((Path(result.stdout.strip()) / source).is_file(), source)


if __name__ == "__main__":
    unittest.main()
