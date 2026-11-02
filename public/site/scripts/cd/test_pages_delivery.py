"""Credential-free checks for exact Pages delivery identity."""
import importlib.util
from email.message import Message
from io import BytesIO
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('pages', Path(__file__).with_name('publish_pages.py'))
assert spec is not None and spec.loader is not None
pages = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pages)


class PagesDeliveryTests(unittest.TestCase):
    def test_old_main_is_refused(self):
        with patch.object(pages.subprocess, 'check_output', return_value='b' * 40 + '\trefs/heads/main\n'):
            with self.assertRaisesRegex(ValueError, 'stale'):
                pages.current_main('a' * 40)

    def test_readback_binds_revision_before_requesting_public_pages(self):
        project = {'canonical_deployment': {'latest_stage': {'status': 'success'},
                   'deployment_trigger': {'metadata': {'commit_hash': 'b' * 40}}}}
        with patch.object(pages, 'api', return_value=project), patch.object(pages, 'urlopen') as http:
            with self.assertRaisesRegex(ValueError, 'checked revision'):
                pages.verify('a' * 40, Path('unused'))
            http.assert_not_called()

    def test_readback_requires_exact_root_and_connect_bytes_and_pages_marker(self):
        sha = 'a' * 40
        project = {'canonical_deployment': {'id': 'deployment', 'url': 'https://deployment.apparent-capture.pages.dev',
                   'latest_stage': {'status': 'success'}, 'deployment_trigger': {'metadata': {'commit_hash': sha}}}}
        class Response(BytesIO):
            headers = {'X-Apparent-Publisher': 'cloudflare-pages',
                       'Cache-Control': 'no-store', 'X-Robots-Tag': 'noindex, nofollow'}
        class MovieResponse(BytesIO):
            headers = {'Content-Type': 'video/mp4'}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'public/site/public/assets/media').mkdir(parents=True, exist_ok=True)
            (root / 'public/site/public/assets/media/Apparent-ghost-demo-v4-lilac.mp4').write_bytes(b'checked movie')
            for name in ['index.html', 'connect/index.html', 'connect/device/index.html', 'connect/shares/index.html']:
                (root / name).parent.mkdir(parents=True, exist_ok=True)
                (root / name).write_bytes(name.encode())
            def fetch(request, timeout):
                if request.full_url.endswith('.mp4'):
                    return MovieResponse(b'checked movie')
                path = {'/': 'index.html', '/connect/': 'connect/index.html', '/connect/device/': 'connect/device/index.html', '/connect/shares/': 'connect/shares/index.html'}
                from urllib.parse import urlsplit
                return Response(path[urlsplit(request.full_url).path].encode())
            with patch.object(pages, 'ROOT', root), patch.object(pages, 'api', return_value=project), patch.object(pages, 'urlopen', side_effect=fetch):
                self.assertEqual(pages.verify(sha, root)['deployment_id'], 'deployment')
                for name in ['connect/device/index.html', 'connect/shares/index.html']:
                    (root / name).write_text('different bytes')
                    with self.assertRaisesRegex(ValueError, 'different bytes'):
                        pages.verify(sha, root)
                    (root / name).write_bytes(name.encode())
                for missing in ['Cache-Control', 'X-Robots-Tag']:
                    with patch.dict(Response.headers, {key: value for key, value in Response.headers.items() if key != missing}, clear=True):
                        with self.assertRaisesRegex(ValueError, 'privacy headers'):
                            pages.verify(sha, root)
            with patch.object(pages, 'api', return_value=project), patch.object(pages, 'urlopen', return_value=BytesIO(b'index.html')):
                with self.assertRaises((AttributeError, ValueError)):
                    pages.verify(sha, root)

    def test_main_stages_only_public_inputs_and_selects_pages_not_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name, data in [('public/site/dist-apparent/404.html', 'not found'),
                               ('public/site/dist-apparent/_routes.json', '{}'),
                               ('public/site/functions/movie.js', 'function source'),
                               ('public/site/lib/metadata.js', 'fixed metadata'),
                               ('public/site/wrangler.pages.jsonc', '{"name":"apparent-capture"}'),
                               ('public/site/wrangler.jsonc', 'unrelated Worker'),
                               ('.env', 'private sentinel')]:
                (root / name).parent.mkdir(parents=True, exist_ok=True)
                (root / name).write_text(data)
            def deploy(argv, cwd, env, check):
                self.assertEqual(argv[1:3], ['pages', 'deploy'])
                self.assertIn('apparent-capture', argv)
                self.assertEqual((cwd / 'wrangler.jsonc').read_text(), '{"name":"apparent-capture"}')
                self.assertEqual(sorted(p.name for p in cwd.iterdir()), ['dist-apparent', 'functions', 'lib', 'wrangler.jsonc'])
                self.assertFalse((cwd / '.env').exists())
                self.assertEqual(env['CLOUDFLARE_ACCOUNT_ID'], pages.ACCOUNT)
            with patch.object(pages, 'ROOT', root), patch.dict(pages.os.environ, {'GITHUB_SHA': 'a' * 40, 'GITHUB_REF': 'refs/heads/main', 'WEBSITE_PAGES_API_TOKEN': 'synthetic'}), patch.object(pages, 'current_main') as current, patch.object(pages.subprocess, 'run', side_effect=deploy) as upload, patch.object(pages, 'verify', return_value={'revision': 'a' * 40}):
                pages.main()
                current.assert_called_once_with('a' * 40)
                upload.assert_called_once()
                # Main can advance after admission while the package is built.
                (root / 'website-receipt.json').unlink()
                upload.reset_mock()
                with patch.object(pages, 'current_main', side_effect=pages.SupersededMain('stale')), patch.object(pages, 'verify_ready') as readback:
                    pages.main()
                upload.assert_not_called()
                readback.assert_not_called()
                self.assertFalse((root / 'website-receipt.json').exists())
                # Transport/auth errors must remain failures, not stale skips.
                with patch.object(pages, 'current_main', side_effect=RuntimeError('git failed')):
                    with self.assertRaisesRegex(RuntimeError, 'git failed'):
                        pages.main()
                upload.assert_not_called()
                (root / 'public/site/dist-apparent/private-link').symlink_to(root, target_is_directory=True)
                upload.reset_mock()
                with self.assertRaisesRegex(ValueError, 'symlink'):
                    pages.main()
                upload.assert_not_called()

    def test_readiness_retry_is_bounded_and_never_masks_integrity_or_auth_failures(self):
        from urllib.error import HTTPError
        for code in (404, 502, 503, 504):
            with self.subTest(code=code), patch.object(pages, 'verify', side_effect=[HTTPError('https://example.test', code, 'temporary', Message(), None), {'deployment_id': 'ready'}]) as verify, patch.object(pages.time, 'sleep') as sleep:
                self.assertEqual(pages.verify_ready('a' * 40, Path('checked')), {'deployment_id': 'ready'})
                self.assertEqual(verify.call_count, 2)
                sleep.assert_called_once_with(1)
        for error in [HTTPError('https://example.test', 403, 'denied', Message(), None), ValueError('different bytes')]:
            with patch.object(pages, 'verify', side_effect=error) as verify, patch.object(pages.time, 'sleep') as sleep:
                with self.assertRaises(type(error)):
                    pages.verify_ready('a' * 40, Path('checked'))
                verify.assert_called_once()
                sleep.assert_not_called()
        with patch.object(pages, 'verify', side_effect=HTTPError('https://example.test', 404, 'not ready', Message(), None)) as verify, patch.object(pages.time, 'sleep') as sleep:
            with self.assertRaises(HTTPError):
                pages.verify_ready('a' * 40, Path('checked'))
            self.assertEqual(verify.call_count, 5)
            self.assertEqual([call.args for call in sleep.call_args_list], [(1,), (2,), (4,), (8,)])

    def test_clanker_publisher_cannot_activate_production(self):
        spec = importlib.util.spec_from_file_location('old', Path(__file__).with_name('publish_site.py'))
        assert spec is not None and spec.loader is not None
        old = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(old)
        with self.assertRaisesRegex(ValueError, 'retired'):
            old.publish('/opt/presentations/releases', 'unused', 'a' * 40, 'unused')


if __name__ == '__main__':
    unittest.main()
