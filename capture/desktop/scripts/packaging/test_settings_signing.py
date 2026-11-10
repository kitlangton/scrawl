"""Exercise nested Electron signing order without invoking a real signer."""
import importlib.util
import json
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("stage_settings", Path(__file__).with_name("stage-settings.py"))
stage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage)


class SigningTests(unittest.TestCase):
    def test_controls_page_is_in_the_installed_settings_closure(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            frontend = root / "capture/desktop/ui/frontend"
            for relative in ("electron/main.cjs", "electron/preload.cjs", "electron/settings.cjs",
                             "dist/settings.html", "dist/onboarding.html", "dist/src/controls/index.html",
                             "dist/onboarding-presentation.cjs", "dist/onboarding-hyprland.cjs", "dist/onboarding-account.cjs",
                             "dist/src/installer/index.html", "dist/pill/index.html",
                             "dist/assets/controls.js", "node_modules/electron/dist/electron"):
                file = frontend / relative
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text(relative)
            for name in ("window-theme.cjs", "tokens.css"):
                file = root / "shared/ui/src" / name
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text(name)
            destination = root / "staged"
            connect = root / "connect/client/typescript"
            (connect / "dist").mkdir(parents=True)
            module = "export const syntheticListing = true;\n"
            (connect / "dist/shares.js").write_text(module)
            (connect / "dist/handoff.js").write_text("synthetic handoff")
            script = root / "capture/desktop/scripts/packaging/stage-settings.py"
            with patch.object(stage, "__file__", str(script)), \
                 patch.object(stage.sys, "argv", [str(script), str(destination)]), \
                 patch.object(stage.sys, "platform", "linux"), \
                 patch.object(stage.subprocess, "run") as build:
                stage.main()
            self.assertTrue(any(call.kwargs.get("cwd") == connect and call.args[0] == ["npm", "run", "build"] for call in build.call_args_list))
            self.assertEqual((destination / "app/connect-client/dist/shares.js").read_text(), module)
            self.assertEqual((destination / "app/connect-client/dist/handoff.js").read_text(), "synthetic handoff")
            self.assertEqual(json.loads((destination / "app/connect-client/package.json").read_text()), {"type": "module"})
            self.assertEqual((destination / "app/dist/src/controls/index.html").read_text(), "dist/src/controls/index.html")
            for relative in ("electron/main.cjs", "electron/preload.cjs", "dist/assets/controls.js",
                             "node_modules/@apparent/ui/src/window-theme.cjs"):
                self.assertTrue((destination / "app" / relative).is_file(), relative)
            self.assertEqual((destination / "app/electron/installer/ui/src/installer/index.html").read_text(), "dist/src/installer/index.html")
            self.assertEqual((destination / "app/dist/pill/index.html").read_text(), "dist/pill/index.html")
            for name in ("presentation", "hyprland", "account"):
                relative = f"dist/onboarding-{name}.cjs"
                self.assertEqual((destination / "app" / relative).read_text(), relative)
            self.assertTrue((destination / "runtime/electron").is_file())

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
