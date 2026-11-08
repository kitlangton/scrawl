"""Publication tests use only disposable synthetic releases, never production."""
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
import fcntl
from unittest import mock

spec = importlib.util.spec_from_file_location('publisher', Path(__file__).with_name('publish_site.py'))
assert spec is not None and spec.loader is not None and spec.origin is not None
module = importlib.util.module_from_spec(spec)
if spec.loader and Path(spec.origin).exists():
    spec.loader.exec_module(module)


class PublishTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=os.environ.get('TMPDIR'))
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.old = self.root / 'apparent-site-baseline'
        self.old.mkdir()
        (self.old / 'index.html').write_text('old')
        (self.old / 'releases').mkdir()
        (self.old / 'releases/app.zip').write_bytes(b'preserve installer')
        (self.old / 'assets').mkdir()
        (self.old / 'assets/old.js').write_text('preserve old tabs')
        self.link = self.root / 'apparent-site'
        self.link.symlink_to(self.old.name)
        self.package = self.root / 'incoming'
        self.package.mkdir()
        (self.package / 'index.html').write_text('new')
        self.sha = 'a' * 40

    def publish(self, **kwargs):
        self.assertTrue(hasattr(module, 'publish'), 'No website CD publisher exists')
        return module.publish(self.root, self.package, self.sha, str(self.old), **kwargs)

    def test_publish_preserves_release_assets_and_records_revision(self):
        result = self.publish(current_sha=lambda: self.sha, verify=lambda release: None)
        active = self.link.resolve()
        self.assertEqual((active / 'index.html').read_text(), 'new')
        self.assertEqual((active / 'releases/app.zip').read_bytes(), b'preserve installer')
        self.assertEqual((active / 'assets/old.js').read_text(), 'preserve old tabs')
        self.assertEqual(result['sha'], self.sha)
        self.assertEqual(result['previous'], str(self.old))

    def test_retired_origin_rejects_publish_and_rollback_before_any_file_is_opened(self):
        retired = Path('/opt/presentations/releases')
        alias = self.root / 'retired-origin-alias'
        alias.symlink_to(retired, target_is_directory=True)
        for root in (retired, alias):
            operations = {
                'publish': lambda: module.publish(root, self.package, self.sha, str(self.old)),
                'rollback': lambda: module.rollback(root, self.root / 'missing-receipt.json'),
            }
            for name, operation in operations.items():
                with self.subTest(root=root, operation=name):
                    with mock.patch.object(Path, 'open', side_effect=AssertionError('Unexpected file access')) as opened:
                        with self.assertRaisesRegex(ValueError, 'retired'):
                            operation()
                        opened.assert_not_called()

    def test_stale_commit_or_baseline_cannot_publish(self):
        with self.assertRaises(ValueError):
            self.publish(current_sha=lambda: 'b' * 40, verify=lambda release: None)
        self.assertEqual(self.link.resolve(), self.old)
        self.link.unlink()
        newer = self.root / 'apparent-site-newer'
        newer.mkdir()
        self.link.symlink_to(newer.name)
        with self.assertRaises(ValueError):
            self.publish(current_sha=lambda: self.sha, verify=lambda release: None)
        self.assertEqual(self.link.resolve(), newer)

    def test_failed_http_verification_rolls_back(self):
        def fail(release):
            raise ValueError('bad public response')
        with self.assertRaises(ValueError):
            self.publish(current_sha=lambda: self.sha, verify=fail)
        self.assertEqual(self.link.resolve(), self.old)

    def test_symlinks_and_private_paths_are_rejected(self):
        (self.package / 'private-pitch').mkdir()
        with self.assertRaises(ValueError):
            self.publish(current_sha=lambda: self.sha, verify=lambda release: None)
        (self.package / 'private-pitch').rmdir()
        (self.package / 'icon.svg').symlink_to(self.old / 'index.html')
        with self.assertRaises(ValueError):
            self.publish(current_sha=lambda: self.sha, verify=lambda release: None)
        self.assertEqual(self.link.resolve(), self.old)

    def test_other_publishers_share_the_existing_lock(self):
        with (self.root / '.apparent-site-publish.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                self.publish(current_sha=lambda: self.sha, verify=lambda release: None)
        self.assertEqual(self.link.resolve(), self.old)


    def test_main_advancing_during_staging_cannot_publish(self):
        revisions = iter([self.sha, 'b' * 40])
        with self.assertRaisesRegex(ValueError, 'advanced during staging'):
            self.publish(current_sha=lambda: next(revisions), verify=lambda release: None)
        self.assertEqual(self.link.resolve(), self.old)

    def test_immutable_asset_collision_cannot_break_existing_tabs(self):
        (self.package / 'assets').mkdir()
        (self.package / 'assets/old.js').write_text('different bytes')
        with self.assertRaisesRegex(ValueError, 'asset name collision'):
            self.publish(current_sha=lambda: self.sha, verify=lambda release: None)
        self.assertEqual(self.link.resolve(), self.old)

    def test_guarded_rollback_refuses_later_release(self):
        result = self.publish(current_sha=lambda: self.sha, verify=lambda release: None)
        receipt = self.root / ('.' + Path(result['active']).name + '.json')
        module.rollback(self.root, receipt)
        self.assertEqual(self.link.resolve(), self.old)
        with self.assertRaisesRegex(ValueError, 'advanced'):
            module.rollback(self.root, receipt)

    def test_failed_verification_does_not_undo_noncooperating_publisher(self):
        newer = self.root / 'apparent-site-newer'
        newer.mkdir()
        def fail(release):
            self.link.unlink()
            self.link.symlink_to(newer.name)
            raise ValueError('outside publisher')
        with self.assertRaisesRegex(ValueError, 'outside publisher'):
            self.publish(current_sha=lambda: self.sha, verify=fail)
        self.assertEqual(self.link.resolve(), newer)


if __name__ == '__main__':
    unittest.main()
