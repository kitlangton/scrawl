"""Exercise nested Electron signing order without invoking a real signer."""
import importlib.util
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
                info.write_bytes(plistlib.dumps({"CFBundleExecutable": name}))
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
