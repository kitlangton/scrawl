"""Synthetic filesystem tests; these never touch the public release directory."""
import fcntl
import importlib.util
import json
import multiprocessing
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location('activation', Path(__file__).with_name('activate-belief-release.py'))
assert spec is not None and spec.loader is not None
activation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(activation)


def activate_child(manifest, root, queue):
    try:
        queue.put(('ok', activation.activate(manifest, root)['active']))
    except (AssertionError, BlockingIOError) as error:
        queue.put(('refused', type(error).__name__))


class PublicationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='belief-publish-test-', dir=os.environ.get('TMPDIR'))
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.baseline = self.root / 'apparent-site-baseline'
        self.baseline.mkdir()
        (self.baseline / 'index.html').write_text('old fixture')
        (self.baseline / 'preserved.js').write_text('unchanged fixture')
        self.candidate = self.root / 'candidate'
        shutil.copytree(self.baseline, self.candidate)
        (self.candidate / 'index.html').write_text('new fixture')
        self.link = self.root / 'apparent-site'
        self.link.symlink_to(self.baseline.name)
        self.manifest = self.root / 'manifest.json'
        self.manifest.write_text(json.dumps({'baseline': str(self.baseline), 'candidate': str(self.candidate),
                                            'before': activation.inventory(self.baseline),
                                            'after': activation.inventory(self.candidate), 'urls': []}))

    def test_activation_and_guarded_rollback(self):
        result = activation.activate(self.manifest, self.root)
        self.assertEqual(self.link.resolve(), Path(result['active']))
        activation.activate(self.manifest, self.root, rollback=True)
        self.assertEqual(self.link.resolve(), self.baseline)
        with self.assertRaises(AssertionError):
            activation.activate(self.manifest, self.root, rollback=True)

    def test_retired_origin_is_rejected_before_any_file_is_opened(self):
        retired = Path('/opt/presentations/releases')
        alias = self.root / 'retired-origin-alias'
        alias.symlink_to(retired, target_is_directory=True)
        for root in (retired, alias):
            for entry in (activation.activate, activation.activate_locked):
                for rollback in (False, True):
                    with self.subTest(root=root, entry=entry.__name__, rollback=rollback):
                        with mock.patch.object(Path, 'open', side_effect=AssertionError('Unexpected file access')) as opened:
                            with self.assertRaisesRegex(ValueError, 'retired'):
                                entry(self.manifest, root, rollback)
                            opened.assert_not_called()

    def test_stale_baseline_is_not_overwritten(self):
        newer = self.root / 'apparent-site-newer'
        newer.mkdir()
        self.link.unlink()
        self.link.symlink_to(newer.name)
        with self.assertRaises(AssertionError):
            activation.activate(self.manifest, self.root)
        self.assertEqual(self.link.resolve(), newer)

    def test_changed_candidate_is_rejected(self):
        (self.candidate / 'index.html').write_text('changed fixture')
        with self.assertRaises(AssertionError):
            activation.activate(self.manifest, self.root)
        self.assertEqual(self.link.resolve(), self.baseline)

    def test_held_publication_lock_refuses_other_process(self):
        queue = multiprocessing.Queue()
        with (self.root / '.apparent-site-publish.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            child = multiprocessing.Process(target=activate_child, args=(self.manifest, self.root, queue))
            child.start()
            child.join(5)
            self.assertFalse(child.is_alive())
            self.assertEqual(queue.get(timeout=1), ('refused', 'BlockingIOError'))
            self.assertEqual(self.link.resolve(), self.baseline)

    def test_competing_publishers_only_one_wins(self):
        queue = multiprocessing.Queue()
        children = [multiprocessing.Process(target=activate_child, args=(self.manifest, self.root, queue)) for _ in range(2)]
        for child in children:
            child.start()
        for child in children:
            child.join(5)
            self.assertFalse(child.is_alive())
        self.assertEqual(sorted(queue.get(timeout=1)[0] for _ in children), ['ok', 'refused'])


if __name__ == '__main__':
    unittest.main()
