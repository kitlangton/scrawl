"""Contract checks for the isolated, manual Mac preview release path."""

from pathlib import Path
import base64
import hashlib
import os
import plistlib
import subprocess
import tempfile
import textwrap
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[4]


class PreviewReleaseTest(unittest.TestCase):
    def test_packager_layout_and_version_with_stubbed_mac_tools(self):
        # A Linux layout test: the real signing, DMG and GUI still need a Mac run.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "capture/desktop/scripts/packaging").mkdir(parents=True)
            (root / "capture/desktop/scripts/release").mkdir(parents=True)
            (root / "bin").mkdir()
            for asset in ("fonts/HostGrotesk-Variable.ttf", "brand/apparent-wordmark.png",
                          "Apparent.icns", "brand/apparent-a.png", "brand/connect-frogs.png"):
                owner = "capture/desktop" if asset == "Apparent.icns" else "shared"
                destination = root / owner / "assets" / asset
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes((ROOT / owner / "assets" / asset).read_bytes())
            (root / "sparkle/Sparkle.framework/Versions/B/XPCServices").mkdir(parents=True)
            (root / "capture/desktop/scripts/packaging/build-mac-preview.sh").write_bytes(
                (ROOT / "capture/desktop/scripts/packaging/build-mac-preview.sh").read_bytes()
            )
            (root / "capture/desktop/scripts/release/setup-sparkle.sh").write_text('#!/bin/sh\nprintf "%s/sparkle\\n" "$PWD"\n')
            (root / "capture/desktop/scripts/release/setup-sparkle.sh").chmod(0o755)
            (root / "bin/cargo").write_text(
                '#!/bin/sh\nmkdir -p target/release\n'
                'printf "test" > target/release/apparent\n'
                'printf "test" > target/release/apparent-helper\n'
            )
            (root / "bin/plutil").write_text('#!/bin/sh\nexit 0\n')
            (root / "bin/codesign").write_text('#!/bin/sh\nexit 0\n')
            (root / "bin/hdiutil").write_text(
                '#!/bin/sh\n'
                'if [ "$1" = create ]; then eval "out=\\${$#}"; printf "fake dmg" > "$out"; fi\n'
            )
            (root / "bin/ditto").write_text(
                '#!/bin/sh\n'
                'if [ "$1" = -c ]; then eval "out=\\${$#}"; printf "fake zip" > "$out"; '
                'else cp -R "$1" "$2"; fi\n'
            )
            for tool in (root / "bin").iterdir():
                tool.chmod(0o755)
            env = dict(os.environ, PATH=f'{root / "bin"}:{os.environ["PATH"]}',
                       APPARENT_PREVIEW_VERSION="0.123.2", APPARENT_PREVIEW_BUILD_NUMBER="123.2",
                       APPARENT_PREVIEW_NOTARIZE="false",
                       APPARENT_PREVIEW_PUBLIC_KEY=base64.b64encode(bytes([1]) * 32).decode())
            result = subprocess.run(["bash", str(root / "capture/desktop/scripts/packaging/build-mac-preview.sh")],
                                    env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            info = plistlib.loads((root / "dist/Apparent Preview.app/Contents/Info.plist").read_bytes())
            self.assertEqual(info["CFBundleIdentifier"], "dev.simonvanlaak.apparent.preview")
            self.assertEqual(info["CFBundleShortVersionString"], "0.123.2")
            self.assertEqual(info["CFBundleVersion"], "123.2")
            self.assertIn("preview-mac-feed/appcast.xml", info["SUFeedURL"])
            self.assertEqual(info["SUPublicEDKey"], env["APPARENT_PREVIEW_PUBLIC_KEY"])
            self.assertTrue((root / "dist/Apparent Preview.app/Contents/Frameworks/Sparkle.framework").is_dir())
            for asset in ("fonts/HostGrotesk-Variable.ttf", "brand/apparent-wordmark.png",
                          "Apparent.icns", "brand/apparent-a.png", "brand/connect-frogs.png"):
                self.assertEqual(
                    (root / "dist/Apparent Preview.app/Contents/Resources" / Path(asset).name).read_bytes(),
                    (root / ("capture/desktop" if asset == "Apparent.icns" else "shared") / "assets" / asset).read_bytes(),
                )
            self.assertTrue((root / "dist/Apparent-Preview-0.123.2-arm64.dmg.sha256").is_file())
            self.assertTrue((root / "dist/Apparent-Preview-0.123.2-arm64.zip").is_file())
            self.assertTrue((root / "dist/preview-staging/Applications").is_symlink())

    def test_app_icon_uses_the_approved_boxed_a_not_the_wordmark_or_brush(self):
        svg = '{http://www.w3.org/2000/svg}'
        brand = ET.parse(ROOT / "shared/assets/brand/apparent-wordmark.svg").getroot()
        app_icon = ET.parse(ROOT / "shared/assets/brand/apparent-app-icon.svg").getroot()
        # Primary icon master: https://tryapparent.com/brand/assets/brand/apparent-icon.svg
        self.assertEqual(app_icon.get("viewBox"), "0 0 512 512")
        self.assertEqual(app_icon.findall(f"{svg}rect")[0].get("fill"), "#020102")
        paths = app_icon.findall(f"{svg}path")
        self.assertEqual(len(paths), 1)
        self.assertEqual(paths[0].get("fill"), "#B7A5D8")
        self.assertEqual(paths[0].get("transform"), "translate(50.08 434.56) scale(.72 -.72)")
        self.assertEqual(paths[0].get("d"), brand.findall(f"{svg}g/{svg}path")[0].get("d"))
        # Pin the reviewed raster too: the vector alone does not reach the Mac bundle.
        raster = (ROOT / "capture/desktop/assets/app-icon.png").read_bytes()
        self.assertEqual(hashlib.sha256(raster).hexdigest(),
                         "328388f2e2743b72acafc7c4fdddafbefb3e8f7e221ab9b9808a00acf3db13de")
        self.assertEqual(hashlib.sha256((ROOT / "capture/desktop/assets/Apparent.icns").read_bytes()).hexdigest(),
                         "f3028c57a513957454d2bb7caf720fcf77358b47dcf1f1c7fc78a39c46e99153")

    def test_mac_status_item_preserves_brand_colors_and_tracks_appearance(self):
        source = (ROOT / "capture/desktop/ui/macos.rs").read_text()
        for asset in ('menubar-icon@2x.png', 'menubar-light@2x.png'):
            self.assertIn(f'include_bytes!("../../../capture/desktop/assets/{asset}")', source)
        self.assertIn('image.setTemplate(false)', source)
        self.assertIn('button.effectiveAppearance()', source)
        self.assertIn('bestMatchFromAppearancesWithNames:', source)
        self.assertIn('addObserver:', source)
        self.assertIn('removeObserver:', source)
        self.assertIn('observeValueForKeyPath:ofObject:change:context:', source)
        self.assertEqual(hashlib.sha256((ROOT / "capture/desktop/assets/menubar-light@2x.png").read_bytes()).hexdigest(),
                         "4b83f745dde9a9de1af2dc439983afd14a93117278e686cea9cb52745e06e4a0")
        self.assertEqual(hashlib.sha256((ROOT / "capture/desktop/assets/menubar-icon@2x.png").read_bytes()).hexdigest(),
                         "4714f8ebbe8dd7816c7f7ab07ad98daa61fb4eee5fc3ed900af8f012b452dc07")

    def test_helper_embedded_identity_follows_preview_identity(self):
        script = (ROOT / "capture/desktop/scripts/packaging/build-mac-preview.sh").read_text()
        build = (ROOT / "build.rs").read_text()
        self.assertIn("APPARENT_HELPER_BUNDLE_ID=dev.simonvanlaak.apparent.preview.helper", script)
        self.assertIn('rerun-if-env-changed=APPARENT_HELPER_BUNDLE_ID', build)
        self.assertIn('env::var("APPARENT_HELPER_BUNDLE_ID")', build)

    def test_preview_cannot_replace_production_cli_or_login_item(self):
        script = (ROOT / "capture/desktop/scripts/packaging/build-mac-preview.sh").read_text()
        cli = (ROOT / "composition/command_line.rs").read_text()
        settings = (ROOT / "capture/desktop/settings/settings.rs").read_text()
        self.assertIn("APPARENT_PREVIEW_BUILD=1", script)
        self.assertIn('option_env!("APPARENT_PREVIEW_BUILD")', cli)
        self.assertIn('option_env!("APPARENT_PREVIEW_BUILD")', settings)

    def test_preview_bundle_has_its_own_sparkle_feed(self):
        script = (ROOT / "capture/desktop/scripts/packaging/build-mac-preview.sh").read_text()
        self.assertIn("dev.simonvanlaak.apparent.preview", script)
        self.assertIn("Apparent Preview.app", script)
        self.assertNotIn("dev.kitlangton.apparent", script)
        self.assertIn("Sparkle.framework", script)
        self.assertIn("SUFeedURL", script)
        self.assertIn("preview-mac-feed/appcast.xml", script)
        self.assertIn("APPARENT_PREVIEW_PUBLIC_KEY", script)
        self.assertNotIn("apparent.kitlangton.com/appcast.xml", script)
        self.assertIn('APPARENT_PREVIEW_NOTARIZE:-false', script)
        self.assertNotIn("site/public", script)
        self.assertIn("codesign --verify", script)
        self.assertIn("hdiutil verify", script)

    def test_manual_release_uses_unique_version_and_publishes_only_after_build(self):
        workflow = (ROOT / ".github/workflows/release-mac-preview.yml").read_text()
        self.assertIn("workflow_dispatch:", workflow)
        self.assertNotIn("  push:", workflow)
        self.assertIn("github.ref == 'refs/heads/main'", workflow)
        self.assertIn("runs-on: macos-latest", workflow)
        self.assertIn("github.run_number", workflow)
        self.assertIn("github.run_attempt", workflow)
        self.assertIn("--prerelease", workflow)
        self.assertIn("--latest=false", workflow)
        self.assertIn("gh release create", workflow)
        self.assertLess(workflow.index("build-mac-preview.sh"), workflow.index("gh release create"))
        self.assertIn("SPARKLE_PREVIEW_PRIVATE_KEY", workflow)
        self.assertIn("SPARKLE_PREVIEW_PUBLIC_KEY", workflow)
        self.assertIn("sign_update", workflow)
        self.assertIn("swift capture/desktop/scripts/release/verify-preview-sparkle-key.swift", workflow)
        self.assertIn("capture/desktop/scripts/release/preview_appcast.py", workflow)
        self.assertLess(workflow.index("gh release create"), workflow.index("capture/desktop/scripts/release/preview_appcast.py"))
        self.assertIn("preview-mac-feed", workflow)
        self.assertIn('tag_args=(--target "$GITHUB_SHA")', workflow)
        self.assertNotIn("--clobber", workflow)
        self.assertNotIn("release-app.sh", workflow)

    def test_existing_tag_must_match_run_commit_and_not_have_a_release(self):
        # A maintainer may pre-create the tag when GITHUB_TOKEN cannot tag a
        # commit that changed workflow files. It must not overwrite a release.
        workflow = (ROOT / ".github/workflows/release-mac-preview.yml").read_text()
        self.assertIn('releases/tags/${tag}', workflow)
        self.assertIn('existing_sha', workflow)
        self.assertIn('"$existing_sha" != "$GITHUB_SHA"', workflow)
        self.assertIn('--verify-tag', workflow)
        self.assertIn(r'\(HTTP 404\)$', workflow)
        self.assertIn('"$tag_type" != "commit"', workflow)
        self.assertIn('Tag moved before appcast publication', workflow)

    def test_publish_tag_guards_with_stubbed_github_api(self):
        workflow = (ROOT / ".github/workflows/release-mac-preview.yml").read_text()
        block = workflow.split("      - name: Publish a uniquely versioned prerelease\n", 1)[1]
        block = block.split("      - name: Publish the signed preview appcast", 1)[0]
        script = textwrap.dedent(block.split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "bin").mkdir()
            (root / "dist").mkdir()
            stub = root / "bin/gh"
            stub.write_text(textwrap.dedent('''\
                #!/usr/bin/env python3
                import os, pathlib, sys
                args = sys.argv[1:]
                scenario = os.environ['SCENARIO']
                if args[:2] == ['release', 'create']:
                    pathlib.Path(os.environ['CREATED']).write_text(' '.join(args))
                    sys.exit(0)
                if args[0] == 'api' and '/releases/tags/' in args[1]:
                    if scenario == 'existing_release':
                        sys.exit(0)
                    print('gh: Not Found (HTTP 404)', file=sys.stderr)
                    sys.exit(1)
                if args[0] == 'api' and '/git/ref/tags/' in args[1]:
                    if scenario == 'api_failure':
                        print('HTTP 403: Forbidden', file=sys.stderr)
                        sys.exit(1)
                    if scenario == 'missing':
                        print('gh: Not Found (HTTP 404)', file=sys.stderr)
                        sys.exit(1)
                    kind = 'tag' if scenario == 'annotated' else 'commit'
                    sha = 'wrong' if scenario == 'wrong' else os.environ['GITHUB_SHA']
                    print(kind + ' ' + sha)
                    sys.exit(0)
                sys.exit(2)
            '''))
            stub.chmod(0o755)
            for scenario, allowed in (("matching", True), ("matching_signed", True), ("missing", True),
                                      ("existing_release", False), ("wrong", False),
                                      ("annotated", False), ("api_failure", False)):
                created = root / "created"
                created.unlink(missing_ok=True)
                env = dict(os.environ, PATH=f'{root / "bin"}:{os.environ["PATH"]}',
                           SCENARIO=scenario, CREATED=str(created), RUNNER_TEMP=tmp,
                           GITHUB_REPOSITORY="tryapparent/apparent", GITHUB_SHA="abc123",
                           GITHUB_RUN_NUMBER="3", GITHUB_RUN_ATTEMPT="1")
                env['APPARENT_PREVIEW_NOTARIZE'] = 'true' if scenario == 'matching_signed' else 'false'
                result = subprocess.run(["bash", "-c", script], cwd=root, env=env,
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode == 0, allowed, (scenario, result.stderr))
                self.assertEqual(created.exists(), allowed, scenario)
                if allowed:
                    args = created.read_text()
                    self.assertIn("--verify-tag" if scenario.startswith("matching") else "--target abc123", args)
                    self.assertIn("notarized by Apple" if scenario == "matching_signed" else "not notarized", args)


if __name__ == "__main__":
    unittest.main()
