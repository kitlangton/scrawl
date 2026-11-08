import base64
import importlib.util
import io
import json
import os
import subprocess
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("apparent_plugin", ROOT / "__init__.py",
                                            submodule_search_locations=[str(ROOT)])
plugin = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = plugin
spec.loader.exec_module(plugin)
from apparent_plugin.adapter import Adapter
from apparent_plugin.evidence import result
from apparent_plugin.process import AdapterError, execute


class Context:
    def __init__(self, command="/installed/apparent"):
        self.command = command
        self.tools = {}
        self.hooks = {}
        self.skills = {}

    def get_config(self, name, default=None):
        return self.command

    def register_tool(self, **entry):
        self.tools[entry["name"]] = entry

    def register_skill(self, name, path, **kwargs):
        self.skills[name] = path

    def register_hook(self, name, callback):
        self.hooks[name] = callback


class InspectionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.bundle = Path(self.temp.name) / "test.apparent"
        self.bundle.mkdir()
        Image.new("RGB", (200, 100), "red").save(self.bundle / "contact-sheet.png")
        self.manifest = {"contactSheet": "contact-sheet.png", "keyframes": []}
        self.adapter = Adapter("/installed/apparent")

    def inspect_local(self, manifest=None, frame=None):
        with patch("apparent_plugin.adapter.execute", side_effect=[
            "Fix the red button", json.dumps(self.manifest if manifest is None else manifest),
        ]):
            return self.adapter.inspect(str(self.bundle), frame)

    def test_local_inspection_returns_instructions_and_visible_pixels_without_mutation(self):
        before = (self.bundle / "contact-sheet.png").read_bytes()
        value = self.inspect_local()
        self.assertTrue(value["_multimodal"])
        self.assertIn("Fix the red button", value["content"][0]["text"])
        with Image.open(io.BytesIO(base64.b64decode(value["content"][1]["image_url"]["url"].split(",")[1]))) as image:
            red, green, blue = image.getpixel((100, 50))
            self.assertGreater(red, 240)
            self.assertLess(green + blue, 20)
        self.assertEqual((self.bundle / "contact-sheet.png").read_bytes(), before)

    def test_keyframe_selection_uses_numbered_manifest_entry(self):
        (self.bundle / "frames").mkdir()
        Image.new("RGB", (80, 100), "blue").save(self.bundle / "frames/frame-01.png")
        self.manifest["keyframes"] = [{"file": "frames/frame-01.png"}]
        value = self.inspect_local(frame=1)
        self.assertIn("frames/frame-01.png", value["text_summary"])
        with self.assertRaises(AdapterError) as error:
            self.inspect_local(frame=2)
        self.assertEqual(error.exception.code, "invalid_frame")

    def test_invalid_sources_and_frames_cannot_run_cli(self):
        with patch("apparent_plugin.adapter.execute") as run:
            for source in ("latest", "relative.apparent", "--help", "https://other.test/capture", "", None):
                with self.subTest(source=source), self.assertRaises(AdapterError):
                    self.adapter.inspect(source)
            for frame in (0, -1, True, "1", 1.5):
                with self.subTest(frame=frame), self.assertRaises(AdapterError):
                    self.adapter.inspect(str(self.bundle), frame)
        run.assert_not_called()

    def test_bad_manifest_and_missing_visuals_preserve_transcript(self):
        for manifest in (None, [], {"contactSheet": 5}, {"contactSheet": "missing.png"}):
            value = self.inspect_local({} if manifest is None else manifest)
            self.assertIsInstance(value, str)
            self.assertIn("Fix the red button", value)
            self.assertIn("Visual evidence unavailable", value)
        for failure in ("not json", AdapterError("cli_failed", "manifest failed")):
            with patch("apparent_plugin.adapter.execute", side_effect=["Fix spelling", failure]):
                value = self.adapter.inspect(str(self.bundle))
            self.assertIn("Fix spelling", value)
            self.assertIn("Visual evidence unavailable", value)

    def test_image_reader_rejects_escape_symlinks_and_nonregular_files(self):
        outside = Path(self.temp.name) / "private.png"
        Image.new("RGB", (50, 50), "blue").save(outside)
        (self.bundle / "linked.png").symlink_to(outside)
        (self.bundle / "linked-dir").symlink_to(outside.parent, target_is_directory=True)
        os.mkfifo(self.bundle / "pipe.png")
        for reference in ("../private.png", str(outside), "linked.png", "linked-dir/private.png", "pipe.png", "a\\b.png"):
            with self.subTest(reference=reference):
                value = result("Instructions", self.bundle, reference)
                self.assertIsInstance(value, str)
                self.assertIn("Visual evidence unavailable", value)

    def test_image_limits_bound_input_dimensions_and_returned_pixels(self):
        Image.new("RGB", (2400, 1200)).save(self.bundle / "wide.png")
        value = result("Instructions", self.bundle, "wide.png")
        with Image.open(io.BytesIO(base64.b64decode(value["content"][1]["image_url"]["url"].split(",")[1]))) as image:
            self.assertEqual(image.size, (1600, 800))
        for name, data in (("broken.png", b"broken"), ("large.png", b"x" * (8 * 1024 * 1024 + 1))):
            (self.bundle / name).write_bytes(data)
            self.assertIn("Visual evidence unavailable", result("Instructions", self.bundle, name))
        Image.new("1", (6000, 5000)).save(self.bundle / "dimensions.png")
        self.assertIn("Visual evidence unavailable", result("Instructions", self.bundle, "dimensions.png"))

    def test_connect_uses_fetch_before_inspect_and_cleans_private_snapshot(self):
        capture = "a" * 32
        snapshots = []

        def cli(command, args, **kwargs):
            if args[0] == "fetch":
                self.assertEqual(args[1], capture)
                self.assertEqual(args[-2:], ["--wait", "120"])
                destination = Path(args[args.index("--out") + 1])
                self.assertFalse(destination.exists())
                self.assertEqual(destination.parent.stat().st_mode & 0o777, 0o700)
                snapshots.append(destination.parent)
                destination.mkdir()
                Image.new("RGB", (200, 100), "red").save(destination / "contact-sheet.png")
                return ""
            self.assertTrue(Path(args[1]).is_absolute(), "inspect does not fetch Connect IDs on current main")
            self.assertTrue(Path(args[1]).is_dir())
            return json.dumps(self.manifest) if "--json" in args else "Fix the red button"

        with patch("apparent_plugin.adapter.execute", side_effect=cli) as run:
            value = self.adapter.inspect(capture)
        self.assertTrue(value["_multimodal"])
        self.assertEqual([call.args[1][0] for call in run.call_args_list], ["fetch", "inspect", "inspect"])
        self.assertIn(capture, value["text_summary"])
        self.assertTrue(snapshots)
        self.assertFalse(snapshots[0].exists())
        with Image.open(io.BytesIO(base64.b64decode(value["content"][1]["image_url"]["url"].split(",")[1]))) as image:
            self.assertEqual(image.size, (200, 100))

    def test_connect_download_failure_cleans_snapshot_without_inspection(self):
        snapshots = []

        def failed(command, args, **kwargs):
            snapshots.append(Path(args[args.index("--out") + 1]).parent)
            raise AdapterError("cli_failed", "Read approval needed")

        with patch("apparent_plugin.adapter.execute", side_effect=failed) as run:
            with self.assertRaises(AdapterError):
                self.adapter.inspect("a" * 32)
        self.assertEqual(run.call_count, 1)
        self.assertFalse(snapshots[0].exists())

    def test_connect_accepts_symlinked_temp_root_without_weakening_bundle_guards(self):
        real = Path(self.temp.name).resolve() / "real-scratch"
        real.mkdir(mode=0o700)
        alias = real.parent / "scratch-alias"
        alias.symlink_to(real, target_is_directory=True)
        snapshots = []

        def cli(command, args, **kwargs):
            if args[0] == "fetch":
                destination = Path(args[args.index("--out") + 1])
                # The full CLI refuses any symlink component in its output path.
                self.assertEqual(destination.parent, destination.parent.resolve())
                self.assertEqual(destination.parent.parent, real)
                self.assertEqual(destination.parent.stat().st_mode & 0o777, 0o700)
                snapshots.append(destination.parent)
                destination.mkdir()
                Image.new("RGB", (20, 20), "red").save(destination / "contact-sheet.png")
                return ""
            return json.dumps(self.manifest) if "--json" in args else "Fix the red button"

        with patch.object(tempfile, "tempdir", str(alias)), patch("apparent_plugin.adapter.execute", side_effect=cli):
            value = self.adapter.inspect("a" * 32)
        self.assertTrue(value["_multimodal"])
        self.assertFalse(snapshots[0].exists())
        self.assertTrue(alias.is_symlink())
        self.assertEqual(list(real.iterdir()), [])
        (self.bundle / "linked.png").symlink_to(self.bundle / "contact-sheet.png")
        self.assertIn("Visual evidence unavailable", result("Instructions", self.bundle, "linked.png"))

    def test_registration_is_side_effect_free_and_handler_validates_arguments(self):
        context = Context()
        with patch("apparent_plugin.adapter.execute") as run:
            plugin.register(context)
            handler = context.tools["apparent_inspect"]["handler"]
            for args in ({}, [], {"source": str(self.bundle), "extra": True}):
                value = json.loads(handler(args))
                self.assertEqual(value["error"]["code"], "invalid_arguments")
        run.assert_not_called()
        self.assertEqual(set(context.tools), {"apparent_inspect"})
        self.assertTrue(context.skills["recordings"].is_file())
        self.assertIsNone(context.hooks["pre_llm_call"](user_message="Fix spelling"))
        for message in ("apparent fetch " + "a" * 32, "Connect capture: " + "a" * 32,
                        [{"type": "text", "text": "apparent inspect /test.apparent"}, {"type": "image_url"}]):
            self.assertIn("apparent_inspect", context.hooks["pre_llm_call"](user_message=message)["context"])
        with patch("apparent_plugin.adapter.execute", side_effect=["Fix the red button", json.dumps(self.manifest)]):
            value = handler({"source": str(self.bundle)})
        self.assertTrue(value["_multimodal"])

    def test_cli_is_discovered_only_at_use_and_reports_correct_host_guidance(self):
        context = Context("")
        plugin.register(context)
        with patch("apparent_plugin.adapter.shutil.which", return_value=None), patch("pathlib.Path.is_file", return_value=False):
            value = json.loads(context.tools["apparent_inspect"]["handler"]({"source": str(self.bundle)}))
        self.assertEqual(value["error"]["code"], "setup_required")
        self.assertIn("remote backend", value["error"]["message"])

    def test_real_subprocess_preserves_arguments_and_bounds_failures(self):
        literal = "a path with spaces; $(touch never)"
        self.assertEqual(execute(sys.executable, ["-c", "import sys; print(sys.argv[1])", literal]), literal + "\n")
        for args, options, code in (
            (["-c", "print('x'*100000)"], {"limit": 1024}, "output_limit"),
            (["-c", "import time; time.sleep(10)"], {"timeout": 0.1}, "timeout"),
            (["-c", "import sys; print('read approval needed',file=sys.stderr); sys.exit(2)"], {}, "cli_failed"),
        ):
            with self.subTest(code=code), self.assertRaises(AdapterError) as error:
                execute(sys.executable, args, **options)
            self.assertEqual(error.exception.code, code)

    def test_group_is_signalled_before_its_ownership_anchor_is_reaped(self):
        original_wait, original_kill = subprocess.Popen.wait, os.killpg
        for script, options in [("print('done')", {}), ("print('x'*100000)", {"limit": 1024}),
                                ("import time; time.sleep(10)", {"timeout": 0.1})]:
            events = []
            def wait(child, *args, **kwargs):
                events.append(("wait", child.pid))
                return original_wait(child, *args, **kwargs)
            def kill(group, signal):
                events.append(("kill", group))
                self.assertEqual(os.getpgid(group), group)
                return original_kill(group, signal)
            with patch("apparent_plugin.process.subprocess.Popen.wait", wait), patch("apparent_plugin.process.os.killpg", kill):
                try:
                    execute(sys.executable, ["-c", script], **options)
                except AdapterError:
                    pass
            self.assertEqual([event for event, _ in events], ["kill", "wait"])
            self.assertEqual(events[0][1], events[1][1])

    def test_missing_cli_preserves_unavailable_result(self):
        with self.assertRaises(AdapterError) as error:
            execute("/nonexistent/apparent-test-cli", [])
        self.assertEqual(error.exception.code, "cli_unavailable")

    def test_supervisor_launch_failure_closes_both_status_descriptors(self):
        descriptors = os.pipe()
        with patch("apparent_plugin.process.os.pipe", return_value=descriptors), patch("apparent_plugin.process.subprocess.Popen", side_effect=ValueError("invalid launch")):
            with self.assertRaises(ValueError):
                execute(sys.executable, [])
        for fd in descriptors:
            with self.assertRaises(OSError):
                os.fstat(fd)


if __name__ == "__main__":
    unittest.main()
