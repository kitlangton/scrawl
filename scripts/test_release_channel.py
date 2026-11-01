import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("release_channel", Path(__file__).with_name("release-channel.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
NS = release.NS


def item(build, version, channel=None):
    channel = f"\n            <sparkle:channel>{channel}</sparkle:channel>" if channel else ""
    return f'''
        <item>
            <title>{version}</title>
            <sparkle:version>{build}</sparkle:version>
            <sparkle:shortVersionString>{version}</sparkle:shortVersionString>{channel}
            <enclosure url="https://example.org/releases/Scrawl-{version}-b{build}-arm64.zip" length="123" sparkle:edSignature="sig-{build}"/>
            <sparkle:deltas><enclosure url="https://example.org/releases/Scrawl{build}-1.delta" sparkle:edSignature="delta-{build}"/></sparkle:deltas>
        </item>'''


def feed(*items):
    return f'<?xml version="1.0" standalone="yes"?>\n<rss xmlns:sparkle="{NS}" version="2.0">\n    <channel>\n        <title>Scrawl</title>{"".join(items)}\n    </channel>\n</rss>'


class Args:
    def __init__(self, command="beta", version=None, build=None):
        self.command, self.version, self.build = command, version, build


class VersionTests(unittest.TestCase):
    def test_versions_are_plain(self):
        for version in ["0.3", "0.3.14-beta.1", "0.3.14-b18", "v0.3.14", "0.3.14;exit"]:
            with self.assertRaises(ValueError):
                release.parse_version(version)

    def test_tags(self):
        self.assertEqual(release.parse_tag("v0.3.14"), ("0.3.14", None))
        self.assertEqual(release.parse_tag("v0.3.14-b18"), ("0.3.14", 18))
        for name in ["v0.3.14-beta.2", "v0.3.14-b0", "0.3.14", "release-1"]:
            self.assertIsNone(release.parse_tag(name))

    def test_the_version_in_flight_is_kept_until_asked(self):
        tags = ["v0.3.13", "v0.3.14-b17"]
        self.assertEqual(release.choose_version(None, tags, []), "0.3.14")
        self.assertEqual(release.choose_version("0.3.15", tags, []), "0.3.15")
        self.assertEqual(release.choose_version(None, ["v0.3.13"], [{"version": "0.3.14"}]), "0.3.14")

    def test_a_stable_version_needs_an_explicit_bump(self):
        with self.assertRaisesRegex(ValueError, "--version 0.3.14"):
            release.choose_version(None, ["v0.3.13"], [])
        with self.assertRaisesRegex(ValueError, "already stable"):
            release.choose_version(None, ["v0.3.14", "v0.3.14-b18"], [])
        with self.assertRaisesRegex(ValueError, "older"):
            release.choose_version("0.3.13", ["v0.3.14-b18"], [])

    def test_build_numbers_never_repeat_across_channels_failures_or_tags(self):
        xml = feed(item(16, "0.3.13"), item(18, "0.3.14", "beta"))
        self.assertEqual(release.next_build(xml, [], []), 19)
        self.assertEqual(release.next_build(xml, [{"build": 20}], []), 21)
        self.assertEqual(release.next_build(xml, [], ["v0.3.14-b22"]), 23)


class FeedTests(unittest.TestCase):
    def test_promotion_makes_the_same_item_stable(self):
        xml = feed(item(19, "0.3.14", "beta"), item(18, "0.3.14", "beta"), item(16, "0.3.13"))
        promoted = release.promoted_feed(xml, 18)
        self.assertEqual(release.item_channels(promoted), {19: "beta", 18: None, 16: None})
        # Only the channel line goes; signatures, deltas and every other byte stay.
        self.assertEqual(promoted, xml.replace("\n            <sparkle:channel>beta</sparkle:channel>\n            <enclosure url=\"https://example.org/releases/Scrawl-0.3.14-b18",
                                               "\n            <enclosure url=\"https://example.org/releases/Scrawl-0.3.14-b18"))
        self.assertEqual(release.promoted_feed(promoted, 18), promoted)

    def test_promotion_refuses_a_downgrade_or_unknown_build(self):
        xml = feed(item(20, "0.3.15"), item(18, "0.3.14", "beta"))
        with self.assertRaisesRegex(ValueError, "behind"):
            release.promoted_feed(xml, 18)
        with self.assertRaisesRegex(ValueError, "not in the feed"):
            release.promoted_feed(xml, 17)

    def test_one_item_per_build(self):
        with self.assertRaisesRegex(ValueError, "more than one item"):
            release.item_channels(feed(item(18, "0.3.14"), item(18, "0.3.14", "beta")))


class PlanTests(unittest.TestCase):
    xml = feed(item(17, "0.3.14", "beta"), item(16, "0.3.13"))
    built = {"version": "0.3.14", "build": 17, "label": "0.3.14-b17", "commit": "a" * 40,
             "dmg": "Scrawl-0.3.14-b17-arm64.dmg", "zip": "Scrawl-0.3.14-b17-arm64.zip", "sha256": {}}

    def test_a_new_commit_is_the_next_build_of_the_version_in_flight(self):
        plan = release.plan(Args(), self.xml, [self.built], "b" * 40, {"v0.3.13": "x", "v0.3.14-b17": "a" * 40})
        self.assertEqual((plan["version"], plan["build"], plan["label"]), ("0.3.14", 18, "0.3.14-b18"))
        self.assertEqual(plan["dmg"], "Scrawl-0.3.14-b18-arm64.dmg")

    def test_the_same_commit_resumes(self):
        self.assertIs(release.plan(Args(), self.xml, [self.built], "a" * 40, {}), self.built)
        with self.assertRaisesRegex(ValueError, "already build 17"):
            release.plan(Args(version="0.3.15"), self.xml, [self.built], "a" * 40, {})

    def test_a_promoted_commit_stops_instead_of_republishing_its_beta(self):
        promoted = self.xml.replace("<sparkle:channel>beta</sparkle:channel>", "")
        with self.assertRaisesRegex(ValueError, r"0.3.14 \(build 17\) is already stable.*--version 0.3.15"):
            release.plan(Args(), promoted, [self.built], "a" * 40, {})
        with self.assertRaisesRegex(ValueError, "already stable"):
            release.plan(Args(), self.xml, [self.built], "a" * 40, {"v0.3.14": "a" * 40})

    def test_promotion_needs_a_finished_build_and_one_stable_build_per_version(self):
        self.assertIs(release.plan(Args("promote", build=17), self.xml, [self.built], "c" * 40, {}), self.built)
        with self.assertRaisesRegex(ValueError, "No finished"):
            release.plan(Args("promote", build=18), self.xml, [self.built], "c" * 40, {})
        with self.assertRaisesRegex(ValueError, "already stable"):
            release.plan(Args("promote", build=17), self.xml, [self.built], "c" * 40, {"v0.3.14": "d" * 40})


class SiteTests(unittest.TestCase):
    def test_only_our_mac_and_windows_downloads_are_hydrated(self):
        def asset(name, sha="sha256:x"):
            return {"name": name, "browser_download_url": f"https://github.com/dl/{name}", "digest": sha}
        releases = [
            {"tag_name": "v0.3.14-b18", "draft": False, "assets": [
                asset("Scrawl-0.3.14-b18-arm64.dmg"), asset("Scrawl-0.3.14-b18-arm64.zip"),
                asset("Scrawl-0.3.14-b18-windows-x64.zip"), asset("Scrawl-0.3.14-b18-windows-x64.zip.sha256"),
                asset("Scrawl-0.3.14-b18-linux-x86_64.AppImage")]},
            {"tag_name": "v0.3.13", "draft": False, "assets": [
                asset("Scrawl-0.3.13-arm64.dmg"), asset("Scrawl-0.3.13-windows-x64.zip")]},
            # Real linux-preview releases reuse these names with different bytes, 81-91 MB each.
            {"tag_name": "linux-preview-3", "draft": False, "assets": [
                asset("Scrawl-0.3.13-linux-x86_64.AppImage", "sha256:a"), asset("SHA256SUMS", "sha256:a")]},
            {"tag_name": "linux-preview-4", "draft": False, "assets": [
                asset("Scrawl-0.3.13-linux-x86_64.AppImage", "sha256:b"), asset("SHA256SUMS", "sha256:b")]},
            {"tag_name": "v0.3.15-b20", "draft": True, "assets": [asset("Scrawl-0.3.15-b20-arm64.dmg")]},
        ]
        self.assertEqual([name for name, _, _ in release.site_downloads(releases)], [
            "Scrawl-0.3.14-b18-arm64.dmg", "Scrawl-0.3.14-b18-arm64.zip", "Scrawl-0.3.14-b18-windows-x64.zip",
            "Scrawl-0.3.13-arm64.dmg", "Scrawl-0.3.13-windows-x64.zip"])


class CaskTests(unittest.TestCase):
    stable = '''cask "scrawl" do
  version "0.3.13"
  sha256 "old"

  livecheck do
    url "https://scrawl.kitlangton.com/appcast.xml"
    strategy :sparkle, &:short_version
  end
end
'''

    def test_stable_cask_skips_beta_items(self):
        text = release.cask_text(self.stable, "scrawl", "0.3.14", "new")
        self.assertIn('version "0.3.14"', text)
        self.assertIn('sha256 "new"', text)
        self.assertIn("item.channel.nil?", text)
        self.assertEqual(release.cask_text(text, "scrawl", "0.3.14", "new"), text)

    def test_beta_cask_takes_the_label(self):
        text = release.cask_text(self.stable.replace('"scrawl"', '"scrawl@beta"'), "scrawl@beta", "0.3.14-b18", "new")
        self.assertIn('version "0.3.14-b18"', text)
        self.assertNotIn("item.channel.nil?", text)


class JobTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.store = Path(self.directory.name)
        (self.store / "updates").mkdir()
        self.feed = self.store / "updates/appcast.xml"
        self.manifest = {"version": "0.3.14", "build": 18, "zip": "Scrawl-0.3.14-b18-arm64.zip"}
        (self.store / "updates" / self.manifest["zip"]).write_bytes(b"zip")
        self.patch = patch.object(release, "STORE", self.store)
        self.patch.start()

    def tearDown(self):
        self.patch.stop()
        self.directory.cleanup()

    def request(self, action):
        result = self.store / "result.json"
        (self.store / "appcast-request.json").write_text(
            json.dumps({"manifest": self.manifest, "action": action, "result": str(result)}))
        return result

    def test_beta_inserts_only_this_build_on_the_beta_channel(self):
        self.feed.write_text(feed(item(16, "0.3.13")))
        result = self.request("beta")

        def fake(*args, **kwargs):
            if args[0] == "security":
                return "private-key"
            if args[1] == "--help":
                return "--channel"
            if str(args[0]).endswith("generate_appcast"):
                self.assertEqual(kwargs["input"], "private-key")
                self.assertEqual(args[1:9], ("--ed-key-file", "-", "--versions", 18, "--channel", "beta", "--maximum-versions", "0"))
                self.feed.write_text(feed(item(18, "0.3.14", "beta"), item(16, "0.3.13")))
            return ""

        with patch.object(release, "run", side_effect=fake) as run:
            release.appcast_job()
            self.assertTrue(str(run.call_args.args[0]).endswith("sign_update"))
        self.assertEqual(json.loads(result.read_text()), {})

    def test_a_bad_generation_restores_the_feed(self):
        original = feed(item(16, "0.3.13"))
        self.feed.write_text(original)
        result = self.request("beta")

        def fake(*args, **kwargs):
            if str(args[0]).endswith("generate_appcast") and args[1] != "--help":
                # A key mismatch: generate_appcast writes the item without a signature.
                self.feed.write_text(feed(item(18, "0.3.14", "beta"), item(16, "0.3.13")).replace('sparkle:edSignature="sig-18"', ""))
            return "--channel"

        with patch.object(release, "run", side_effect=fake):
            with self.assertRaises(RuntimeError):
                release.appcast_job()
        self.assertEqual(self.feed.read_text(), original)
        self.assertIn("signed enclosure", json.loads(result.read_text())["error"])

    def test_promotion_edits_the_feed_without_regenerating(self):
        self.feed.write_text(feed(item(18, "0.3.14", "beta"), item(16, "0.3.13")))
        result = self.request("promote")
        with patch.object(release, "run", return_value="") as run:
            release.appcast_job()
            self.assertFalse(any(str(c.args[0]).endswith("generate_appcast") for c in run.call_args_list))
        self.assertEqual(release.item_channels(self.feed.read_text()), {18: None, 16: None})
        self.assertEqual(json.loads(result.read_text()), {})

    def test_an_interrupted_submission_resumes_the_same_request(self):
        result = self.store / "result.json"
        request = {"manifest": self.manifest, "action": "beta", "result": str(result)}
        (self.store / "appcast-request.json").write_text(json.dumps(request))
        with patch.object(release, "run") as run, patch.object(release, "wait_for", side_effect=lambda p: p.write_text("{}")):
            release.appcast(self.manifest, "beta")
            self.assertEqual(run.call_count, 1)
            self.assertEqual(run.call_args.args[:2], ("launchctl", "kickstart"))
        self.assertEqual(json.loads((self.store / "appcast-request.json").read_text()), request)


class FileTests(unittest.TestCase):
    def test_immutable_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            a, b = Path(directory) / "a", Path(directory) / "b"
            a.write_bytes(b"signed original")
            release.immutable_copy(a, b)
            release.immutable_copy(a, b)
            a.write_bytes(b"rebuilt")
            with self.assertRaises(ValueError):
                release.immutable_copy(a, b)
            self.assertEqual(b.read_bytes(), b"signed original")

    def test_completed_signing_is_reused_after_an_interrupted_upload(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            (folder / "build-result.json").write_text("{}")
            with patch.object(release.subprocess, "run") as process, patch.object(release, "run") as run:
                process.return_value.returncode = 1  # No leftover launchd job.
                release.gui_build({"version": "0.3.14", "build": 18}, folder)
                run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
