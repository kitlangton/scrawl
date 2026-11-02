"""Host-safe archive boundary tests, discovered by existing cheap push checks."""
import importlib.util
import io
import os
from pathlib import Path
import tarfile
import tempfile
import subprocess
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('mac_cache', ROOT / 'capture/desktop/scripts/mac/cache.py')
cache = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(cache)


class CacheTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.home = self.root / 'source'
        self.dest = self.root / 'restored'
        self.archive = self.root / 'cache.tar'
        self.lock = self.root / 'Cargo.lock'
        self.data = b'synthetic crate bytes'
        self.name = 'demo-1.0.0.crate'
        self.lock.write_text('[[package]]\nname="demo"\nversion="1.0.0"\n'
                             'source="registry+https://github.com/rust-lang/crates.io-index"\n'
                             f'checksum="{cache.digest(self.data)}"\n')
        self.source = self.home / 'registry/cache' / cache.REGISTRY / self.name
        self.source.parent.mkdir(parents=True)
        self.source.write_bytes(self.data)
        (self.home / 'credentials.toml').write_text('secret')

    def archive_entry(self, name, data=None, kind=tarfile.REGTYPE):
        with tarfile.open(self.archive, 'w') as tar:
            entry = tarfile.TarInfo(name)
            entry.type = kind
            entry.size = len(data or b'')
            tar.addfile(entry, io.BytesIO(data or b''))

    def test_roundtrip_private_allowlist(self):
        self.assertTrue(cache.export_cache(self.home, self.archive, self.lock))
        with tarfile.open(self.archive) as tar:
            self.assertEqual(tar.getnames(), [self.name])
        self.assertEqual(self.archive.stat().st_mode & 0o777, 0o600)
        self.assertTrue(cache.import_cache(self.dest, self.archive, self.lock))
        restored = self.dest / 'registry/cache' / cache.REGISTRY / self.name
        self.assertEqual(restored.read_bytes(), self.data)
        self.assertEqual(restored.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.dest.stat().st_mode & 0o777, 0o700)
        with self.assertRaises(ValueError):
            cache.import_cache(self.dest, self.archive, self.lock)

    def test_miss(self):
        self.assertFalse(cache.import_cache(self.dest, self.archive, self.lock))
        self.source.unlink()
        self.assertFalse(cache.export_cache(self.home, self.archive, self.lock))
        self.assertFalse(self.archive.exists())

    def test_corruption_and_wrong_lock(self):
        for data in (b'corrupt', self.data):
            self.archive_entry(self.name, data)
            if data == self.data:
                self.lock.write_text(self.lock.read_text().replace(cache.digest(data), '0' * 64))
            with self.assertRaises(ValueError):
                cache.import_cache(self.dest, self.archive, self.lock)
            self.assertFalse(self.dest.exists())

    def test_unsafe_entries(self):
        for name, kind in (('../escape', tarfile.REGTYPE), ('/absolute', tarfile.REGTYPE),
                           (self.name, tarfile.SYMTYPE), (self.name, tarfile.LNKTYPE),
                           ('credentials.toml', tarfile.REGTYPE), (self.name, tarfile.FIFOTYPE)):
            with self.subTest(name=name, kind=kind):
                self.archive_entry(name, self.data, kind)
                with self.assertRaises(ValueError):
                    cache.import_cache(self.dest, self.archive, self.lock)
                self.assertFalse(self.dest.exists())

    def test_cap_and_source_symlink(self):
        with patch.object(cache, 'LIMIT', 1):
            with self.assertRaises(ValueError):
                cache.export_cache(self.home, self.archive, self.lock)
        self.source.unlink()
        self.source.symlink_to(self.lock)
        with self.assertRaises(ValueError):
            cache.export_cache(self.home, self.archive, self.lock)

    def test_truncated_archive(self):
        self.archive.write_bytes(b'broken')
        with self.assertRaises(tarfile.TarError):
            cache.import_cache(self.dest, self.archive, self.lock)
        self.assertFalse(self.dest.exists())

    def test_fifo_archive_is_rejected_without_blocking(self):
        os.mkfifo(self.archive)
        result = subprocess.run(
            [sys.executable, str(ROOT / 'capture/desktop/scripts/mac/cache.py'),
             'import', '--cargo-home', str(self.dest), '--archive',
             str(self.archive), '--lock', str(self.lock)],
            capture_output=True, text=True, timeout=2)
        self.assertEqual(result.returncode, 1)
        self.assertIn('cache rejected:', result.stderr)
        self.assertFalse(self.dest.exists())

    def test_nonregular_archive_paths_are_rejected(self):
        self.archive.mkdir()
        with self.assertRaisesRegex(ValueError, 'unsafe'):
            cache.import_cache(self.dest, self.archive, self.lock)
        self.archive.rmdir()
        self.archive.symlink_to(self.root / 'missing')
        with self.assertRaisesRegex(ValueError, 'unsafe'):
            cache.import_cache(self.dest, self.archive, self.lock)


if __name__ == '__main__':
    unittest.main()
