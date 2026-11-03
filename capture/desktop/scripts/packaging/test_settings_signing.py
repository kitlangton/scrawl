"""Exercise nested Electron signing order without invoking a real signer."""
import importlib.util
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("stage_settings", Path(__file__).with_name("stage-settings.py"))
stage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage)


class SigningTests(unittest.TestCase):
    def test_overlapping_destinations_refused_before_build_or_removal(self):
        with tempfile.TemporaryDirectory() as temporary:
            settings = Path(temporary) / "settings"
            settings.mkdir()
            marker = settings / "preserve"
            marker.write_text("existing output")
            alias = Path(temporary) / "alias"
            alias.symlink_to(settings, target_is_directory=True)
            for popups in (settings, settings / "nested", settings.parent, alias):
                argv = ["stage-settings", str(settings), "--popups-destination", str(popups)]
                with self.subTest(popups=popups), patch.object(stage.sys, "argv", argv), \
                     patch.object(stage.subprocess, "run") as run:
                    with self.assertRaises(SystemExit):
                        stage.main()
                    run.assert_not_called()
                    self.assertEqual(marker.read_text(), "existing output")

    def test_one_fresh_build_stages_both_surfaces_and_propagates_failures(self):
        # Run the real popup stager against disposable generated output; only
        # dependency installation/build and the OS are synthetic here.
        for failure in (None, "standalone", "build", "popup"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary) / "repo"
                frontend = root / "capture/desktop/ui/frontend"
                script = root / "capture/desktop/scripts/packaging/stage-settings.py"
                script.parent.mkdir(parents=True)
                for name in ("electron/notes/main.cjs", "electron/share/main.cjs",
                             "electron/share/preload.cjs", "electron/share/main.cts",
                             "dist/index.html", "dist/settings.html", "dist/pill/index.html",
                             "dist/onboarding.html", "dist/assets/app.js",
                             "node_modules/electron/dist/electron"):
                    file = frontend / name
                    file.parent.mkdir(parents=True, exist_ok=True)
                    file.write_text("fresh output")
                for name in ("window-theme.cjs", "tokens.css"):
                    file = root / "shared/ui/src" / name
                    file.parent.mkdir(parents=True, exist_ok=True)
                    file.write_text("theme")
                (frontend / "scripts").mkdir()
                shutil.copy2(Path(stage.__file__).parents[2] / "ui/frontend/scripts/popup-entries.mjs",
                             frontend / "scripts/popup-entries.mjs")
                destination = Path(temporary) / "settings"
                popups = Path(temporary) / "popups"
                real_run = subprocess.run
                calls = []

                def run(command, **kwargs):
                    calls.append(command)
                    if command[:3] == ["npm", "run", "build"] and failure == "build":
                        raise subprocess.CalledProcessError(1, command)
                    if command[:2] == ["node", "--input-type=module"]:
                        if failure == "popup":
                            (frontend / "dist/index.html").unlink()
                        return real_run(command, **kwargs, capture_output=True)

                argv = [str(script), str(destination), "--popups-destination", str(popups)]
                if failure == "standalone":
                    argv = argv[:2]
                with patch.object(stage, "__file__", str(script)), \
                     patch.object(stage.sys, "platform", "linux"), \
                     patch.object(stage.sys, "argv", argv), \
                     patch.object(stage.subprocess, "run", side_effect=run):
                    if failure in ("build", "popup"):
                        with self.assertRaises(subprocess.CalledProcessError):
                            stage.main()
                        self.assertFalse(destination.exists())
                    else:
                        stage.main()
                        self.assertTrue((destination / "app/dist/settings.html").is_file())
                        self.assertTrue((destination / "runtime/electron").is_file())
                        if failure == "standalone":
                            self.assertFalse(popups.exists())
                        else:
                            self.assertTrue((popups / "electron/share/main.cjs").is_file())
                            self.assertFalse((popups / "electron/share/main.cts").exists())
                self.assertEqual(sum(c[:2] == ["npm", "ci"] for c in calls), 1)
                self.assertEqual(sum(c == ["npm", "run", "build"] for c in calls), 1)

    def test_principal_executables_wait_for_nested_code(self):
        with tempfile.TemporaryDirectory() as temporary:
            runtime = Path(temporary) / "Electron.app"
            helper = runtime / "Contents/Frameworks/Electron Helper (Renderer).app"
            framework = runtime / "Contents/Frameworks/Electron Framework.framework"
            principals = []
            for bundle in (runtime, helper, framework):
                bundle.mkdir(parents=True, exist_ok=True)
                name = bundle.stem
                if bundle.suffix == ".app":
                    info = bundle / "Contents/Info.plist"
                    binary = bundle / "Contents/MacOS" / name
                else:
                    info = bundle / "Versions/A/Resources/Info.plist"
                    binary = bundle / "Versions/A" / name
                    (bundle / "Resources").symlink_to("Versions/A/Resources")
                    (bundle / name).symlink_to("Versions/A/" + name)
                info.parent.mkdir(parents=True, exist_ok=True)
                info.write_bytes(plistlib.dumps({} if bundle == helper else {"CFBundleExecutable": name}))
                binary.parent.mkdir(parents=True, exist_ok=True)
                binary.write_bytes(b"\xcf\xfa\xed\xfe synthetic Mach-O")
                principals.append(binary)
            library = framework / "Versions/A/Libraries/libfixture.dylib"
            library.parent.mkdir()
            library.write_bytes(b"\xcf\xfa\xed\xfe synthetic library")
            signed = []

            def signer(command, check):
                target = Path(command[-1])
                if "--verify" in command:
                    self.assertEqual(command, ["codesign", "--verify", "--deep", "--strict", str(runtime)])
                    self.assertEqual(signed[-1], runtime)
                    return
                # codesign treats the principal path as its bundle; this failed in CI.
                self.assertNotIn(target, principals)
                if target == framework:
                    self.assertIn(library, signed)
                if target == runtime:
                    self.assertIn(helper, signed)
                    self.assertIn(framework, signed)
                signed.append(target)

            with patch.object(stage.subprocess, "run", side_effect=signer):
                stage.sign_runtime(runtime, ["codesign", "--force", "--sign", "-"], Path("Settings.entitlements"))
            self.assertEqual(set(signed), {library, helper, framework, runtime})
            with patch.object(stage.subprocess, "run", side_effect=subprocess.CalledProcessError(1, "codesign")) as run:
                with self.assertRaises(subprocess.CalledProcessError):
                    stage.sign_runtime(runtime, ["codesign", "--sign", "-"], Path("Settings.entitlements"))
                self.assertEqual(run.call_count, 1)


if __name__ == "__main__":
    unittest.main()
