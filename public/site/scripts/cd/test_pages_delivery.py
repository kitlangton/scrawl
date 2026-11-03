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
    def test_publisher_checks_deployed_revision_instead_of_requiring_main_tip(self):
        project = {'canonical_deployment': {'deployment_trigger': {'metadata': {'commit_hash': 'b' * 40}}}}
        with patch.object(pages, 'api', return_value=project), patch.dict(pages.os.environ, {'GITHUB_REPOSITORY': 'example/project'}):
            with patch.object(pages, 'publication_needed', return_value=True) as order:
                pages.current_main('c' * 40)
                order.assert_called_once_with('example/project', 'c' * 40, 'b' * 40)
            with patch.object(pages, 'publication_needed', return_value=False):
                with self.assertRaises(pages.SupersededMain):
                    pages.current_main('a' * 40)
            with patch.object(pages, 'publication_needed', side_effect=RuntimeError('API failed')):
                with self.assertRaisesRegex(RuntimeError, 'API failed'):
                    pages.current_main('c' * 40)

    def test_readback_binds_revision_before_requesting_public_pages(self):
        project = {'canonical_deployment': {'latest_stage': {'status': 'success'},
                   'deployment_trigger': {'metadata': {'commit_hash': 'b' * 40}}}}
        with patch.object(pages, 'api', return_value=project), patch.object(pages, 'urlopen') as http:
            with self.assertRaisesRegex(ValueError, 'checked revision'):
                pages.verify('a' * 40, Path('unused'))
            http.assert_not_called()

    def fixture(self, root):
        for name, data in [('public/site/dist-apparent/index.html', 'source homepage'),
                           ('public/site/dist-apparent/404.html', 'not found'),
                           ('public/site/dist-apparent/_routes.json', '{}'),
                           ('public/site/dist-apparent/_worker.js', 'compiled worker'),
                           ('public/site/dist-apparent/assets/index-test.js', 'compiled source'),
                           ('public/site/dist-apparent/connect/index.html', 'connect'),
                           ('public/site/dist-apparent/connect/device/index.html', 'device'),
                           ('public/site/dist-apparent/connect/shares/index.html', 'shares'),
                           ('public/site/public/assets/media/Apparent-ghost-demo-v4-lilac.mp4', 'checked movie'),
                           ('public/site/functions/movie.js', 'source must not reach publisher'),
                           ('public/site/wrangler.pages.jsonc', '{"name":"apparent-capture"}'),
                           ('public/site/wrangler.jsonc', 'unrelated Worker'),
                           ('.env', 'private sentinel')]:
            (root / name).parent.mkdir(parents=True, exist_ok=True)
            (root / name).write_text(data)
        with patch.object(pages, 'ROOT', root):
            pages.seal('a' * 40)
        return root / '.build/website'

    def test_sealed_artifact_refuses_wrong_revision_tampering_extra_files_and_links(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact = self.fixture(root)
            with self.assertRaisesRegex(ValueError, 'revision'):
                pages.verify_artifact('b' * 40, artifact)
            target = artifact / 'dist-apparent/index.html'
            original = target.read_bytes()
            target.write_text('tampered')
            with self.assertRaisesRegex(ValueError, 'checksum'):
                pages.verify_artifact('a' * 40, artifact)
            target.write_bytes(original)
            extra = artifact / 'dist-apparent/extra.js'
            extra.write_text('unexpected')
            with self.assertRaisesRegex(ValueError, 'checksum'):
                pages.verify_artifact('a' * 40, artifact)
            extra.unlink()
            extra.symlink_to(root, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, 'symlink'):
                pages.verify_artifact('a' * 40, artifact)
            extra.unlink()
            self.assertEqual(pages.verify_artifact('a' * 40, artifact)['revision'], 'a' * 40)
            with patch.object(pages, 'ROOT', root), self.assertRaises(FileExistsError):
                pages.seal('a' * 40)

    def test_readback_requires_exact_html_assets_movie_and_privacy_at_both_origins(self):
        from urllib.parse import urlsplit
        sha = 'a' * 40
        project = {'canonical_deployment': {'id': 'deployment', 'url': 'https://deployment.apparent-capture.pages.dev',
                   'latest_stage': {'status': 'success'}, 'deployment_trigger': {'metadata': {'commit_hash': sha}}}}
        class Response(BytesIO):
            headers = {'X-Apparent-Publisher': 'cloudflare-pages', 'Cache-Control': 'no-store',
                       'X-Robots-Tag': 'noindex, nofollow', 'Content-Type': 'video/mp4',
                       'Set-Cookie': 'private-cookie-sentinel'}
        with tempfile.TemporaryDirectory() as directory:
            artifact = self.fixture(Path(directory))
            package = artifact / 'dist-apparent'
            paths = {'/': 'index.html', '/connect/': 'connect/index.html', '/connect/device/': 'connect/device/index.html',
                     '/connect/shares/': 'connect/shares/index.html', '/assets/index-test.js': 'assets/index-test.js'}
            visited = set()
            bad_path = None
            bad_origin = None
            def fetch(request, timeout):
                url = urlsplit(request.full_url)
                visited.add(url.netloc)
                if url.path == bad_path and (bad_origin is None or url.netloc == bad_origin):
                    return Response(b'unexpected response body sentinel')
                if url.path.endswith('.mp4'):
                    return Response(b'checked movie')
                return Response((package / paths[url.path]).read_bytes())
            with patch.object(pages, 'api', return_value=project), patch.object(pages, 'urlopen', side_effect=fetch):
                self.assertEqual(pages.verify(sha, package)['deployment_id'], 'deployment')
                self.assertEqual(visited, {'deployment.apparent-capture.pages.dev', 'app.tryapparent.com'})
                for bad_path in paths:
                    with self.assertRaisesRegex(ValueError, 'different bytes') as error:
                        pages.verify(sha, package)
                    self.assertIn('actual_sha256', str(error.exception))
                    self.assertIn('expected_sha256', str(error.exception))
                    self.assertIn('content_encoding', str(error.exception))
                    self.assertNotIn('unexpected response body sentinel', str(error.exception))
                    self.assertNotIn('private-cookie-sentinel', str(error.exception))
                bad_path = '/Apparent-ghost-demo-v4-lilac.mp4'
                with self.assertRaisesRegex(ValueError, 'different movie'):
                    pages.verify(sha, package)
                bad_path = None
                bad_origin = 'app.tryapparent.com'
                bad_path = '/'
                with self.assertRaises(pages.CanonicalPropagationPending):
                    pages.verify(sha, package)
                bad_path = None
                for missing in ['X-Apparent-Publisher', 'Cache-Control', 'X-Robots-Tag']:
                    with patch.dict(Response.headers, {key: value for key, value in Response.headers.items() if key != missing}, clear=True):
                        with self.assertRaises(ValueError):
                            pages.verify(sha, package)

    def test_main_deploys_only_sealed_artifact_without_rebuilding_functions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact = self.fixture(root)
            # The source tree may disappear entirely after the verified build.
            import shutil
            shutil.rmtree(root / 'public/site')
            def deploy(argv, cwd, env, check):
                self.assertEqual(argv[1:3], ['pages', 'deploy'])
                self.assertIn('--no-bundle', argv)
                self.assertIn('apparent-capture', argv)
                self.assertEqual((cwd / 'wrangler.jsonc').read_text(), '{"name":"apparent-capture"}')
                self.assertEqual(sorted(p.name for p in cwd.iterdir()), ['dist-apparent', 'wrangler.jsonc'])
                self.assertEqual((cwd / 'dist-apparent/_worker.js').read_text(), 'compiled worker')
                self.assertFalse((cwd / '.env').exists())
                self.assertEqual(env['CLOUDFLARE_ACCOUNT_ID'], pages.ACCOUNT)
            with patch.object(pages, 'ROOT', root), patch.dict(pages.os.environ, {'GITHUB_SHA': 'a' * 40, 'GITHUB_REF': 'refs/heads/main', 'WEBSITE_PAGES_API_TOKEN': 'synthetic'}), patch.object(pages, 'current_main') as current, patch.object(pages.subprocess, 'run', side_effect=deploy) as upload, patch.object(pages, 'verify', return_value={'revision': 'a' * 40}):
                pages.main()
                current.assert_called_once_with('a' * 40)
                upload.assert_called_once()
                (root / 'website-receipt.json').unlink()
                upload.reset_mock()
                with patch.object(pages, 'current_main', side_effect=pages.SupersededMain('stale')), patch.object(pages, 'verify_ready') as readback:
                    pages.main()
                upload.assert_not_called()
                readback.assert_not_called()
                self.assertFalse((root / 'website-receipt.json').exists())
                with patch.object(pages, 'current_main', side_effect=RuntimeError('git failed')):
                    with self.assertRaisesRegex(RuntimeError, 'git failed'):
                        pages.main()
                upload.assert_not_called()
                (artifact / 'dist-apparent/index.html').write_text('changed after verification')
                with self.assertRaisesRegex(ValueError, 'checksum'):
                    pages.main()
                upload.assert_not_called()

    def test_readiness_retry_is_bounded_and_never_masks_integrity_or_auth_failures(self):
        from urllib.error import HTTPError
        with patch.object(pages, 'verify', side_effect=[pages.CanonicalPropagationPending('canonical bytes differ'), {'deployment_id': 'ready'}]) as verify, patch.object(pages.time, 'sleep') as sleep:
            self.assertEqual(pages.verify_ready('a' * 40, Path('checked')), {'deployment_id': 'ready'})
            self.assertEqual(verify.call_count, 2)
            sleep.assert_called_once_with(1)
        with patch.object(pages, 'verify', side_effect=pages.CanonicalPropagationPending('canonical bytes differ')) as verify, patch.object(pages.time, 'sleep') as sleep:
            with self.assertRaises(pages.CanonicalPropagationPending):
                pages.verify_ready('a' * 40, Path('checked'))
            self.assertEqual(verify.call_count, 5)
            self.assertEqual([call.args for call in sleep.call_args_list], [(1,), (2,), (4,), (8,)])
        for code in (404, 500, 502, 503, 504):
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
        for code in (404, 500):
            with self.subTest(persistent=code), patch.object(pages, 'verify', side_effect=HTTPError('https://example.test', code, 'not ready', Message(), None)) as verify, patch.object(pages.time, 'sleep') as sleep:
                with self.assertRaises(HTTPError):
                    pages.verify_ready('a' * 40, Path('checked'))
                self.assertEqual(verify.call_count, 5)
                self.assertEqual([call.args for call in sleep.call_args_list], [(1,), (2,), (4,), (8,)])

if __name__ == '__main__':
    unittest.main()
