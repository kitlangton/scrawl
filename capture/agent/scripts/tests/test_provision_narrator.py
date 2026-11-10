import hashlib
import importlib.util
import io
import os
from pathlib import Path
import tempfile
import subprocess
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("provision_narrator", Path(__file__).parents[1] / "provision-narrator.py")
assert SPEC is not None and SPEC.loader is not None
p = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(p)


class ProvisionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.fd = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
        self.addCleanup(os.close, self.fd)
        self.payload = b"trusted fixture"
        self.digest = hashlib.sha256(self.payload).hexdigest()

    def asset(self, data=None):
        with patch.object(p, "urlopen", create=True, return_value=io.BytesIO(self.payload if data is None else data)) as request:
            p.ensure_asset(self.fd, "model.onnx", "https://example.org/model", len(self.payload), self.digest)
            return request.call_count

    def test_readiness_missing_cache_does_not_create_directories(self):
        missing = self.root / "missing/cache"
        result = subprocess.run(["node", str(Path(__file__).parents[1] / "provision-narrator.ts"),
                                 "--check", "--cache", str(missing)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn('"status":"unavailable"', result.stdout)
        self.assertFalse((self.root / "missing").exists())

    def test_typescript_policy_contracts(self):
        subprocess.run(["node", "--test", str(Path(__file__).with_name("provision-narrator.test.ts"))], check=True)

    def test_check_ignores_interrupted_partials(self):
        (self.root / ".narrator-interrupted.partial").write_bytes(b"partial")
        self.assertEqual(p.inspect_cache(self.root), {"assets": False, "worker": False})
        self.assertTrue((self.root / ".narrator-interrupted.partial").exists())
        self.assertEqual(self.asset(), 1)
        self.assertTrue((self.root / ".narrator-interrupted.partial").exists())

    def test_download_and_verify_existing_without_network(self):
        self.assertEqual(self.asset(), 1)
        self.assertEqual((self.root / "model.onnx").read_bytes(), self.payload)
        self.assertEqual(self.asset(), 0)
        (self.root / "model.onnx").write_bytes(b"corrupted")
        with self.assertRaisesRegex(ValueError, "integrity"):
            self.asset()

    def test_bad_download_leaves_no_artifact(self):
        for data in [b"bad", b"X" * len(self.payload), b"X" * (len(self.payload) + 1)]:
            with self.assertRaises(ValueError): self.asset(data)
            self.assertEqual(list(self.root.iterdir()), [])

    def test_rejects_asset_symlink_and_preserves_target(self):
        outside = self.root / "outside"
        outside.write_bytes(self.payload)
        (self.root / "model.onnx").symlink_to(outside)
        with self.assertRaises((ValueError, OSError)): self.asset()
        self.assertEqual(outside.read_bytes(), self.payload)

    def test_cache_rejects_symlink_ancestor_and_repo(self):
        real = self.root / "real"
        real.mkdir()
        (self.root / "link").symlink_to(real, target_is_directory=True)
        with self.assertRaises((ValueError, OSError)): p.open_cache(self.root / "link/cache")
        with self.assertRaises(ValueError): p.open_cache(Path(__file__).resolve().parents[3] / "cache")
        self.assertEqual(list(real.iterdir()), [])

    def test_cache_fd_pins_directory_during_rename(self):
        cache = self.root / "cache"
        fd = p.open_cache(cache)
        self.addCleanup(os.close, fd)
        cache.rename(self.root / "original")
        cache.symlink_to(self.root / "real", target_is_directory=True)
        with patch.object(p, "urlopen", create=True, return_value=io.BytesIO(self.payload)):
            p.ensure_asset(fd, "model.onnx", "https://example.org/model", len(self.payload), self.digest)
        self.assertEqual((self.root / "original/model.onnx").read_bytes(), self.payload)
        self.assertFalse((self.root / "real").exists())

    def test_rejects_non_https(self):
        with self.assertRaises(ValueError):
            p.ensure_asset(self.fd, "model.onnx", "http://example.org/model", len(self.payload), self.digest)

    def test_rejects_other_source_repository(self):
        (self.root / ".git").write_text("gitdir: elsewhere")
        with self.assertRaises(ValueError): p.open_cache(self.root)
        with self.assertRaises(ValueError): p.open_cache(self.root / "cache")
        self.assertFalse((self.root / "cache").exists())

    def test_timeout_removes_partial(self):
        with patch.object(p.time, "monotonic", side_effect=[0, 901]):
            with self.assertRaises(TimeoutError): self.asset()
        self.assertEqual(list(self.root.iterdir()), [])

    def test_interrupted_response_removes_only_own_partial_then_retry_succeeds(self):
        class Interrupted(io.BytesIO):
            def read(self, size=-1):
                raise ConnectionResetError("interrupted fixture")
        with patch.object(p, "urlopen", return_value=Interrupted()):
            with self.assertRaises(ConnectionResetError):
                p.ensure_asset(self.fd, "model.onnx", "https://example.org/model", len(self.payload), self.digest)
        self.assertEqual(list(self.root.iterdir()), [])
        self.assertEqual(self.asset(), 1)

    def test_refuses_https_downgrade(self):
        with self.assertRaises(ValueError):
            p.HTTPSRedirects().redirect_request(None, None, 302, "", {}, "http://example.org")

    def test_hash_checked_even_when_size_matches(self):
        (self.root / "model.onnx").write_bytes(b"X" * len(self.payload))
        with self.assertRaisesRegex(ValueError, "SHA256"):
            self.asset()

    def test_worker_install_is_atomic_and_does_not_follow_symlink(self):
        outside = self.root / "source"
        outside.write_bytes(b"untouched")
        binary = self.root / "built"
        binary.write_bytes(b"worker")
        (self.root / "apparent-narrator").symlink_to(outside)
        p.install_worker(self.fd, binary)
        self.assertEqual(outside.read_bytes(), b"untouched")
        self.assertEqual((self.root / "apparent-narrator").read_bytes(), b"worker")
        self.assertEqual((self.root / "apparent-narrator").stat().st_mode & 0o777, 0o700)
        self.assertFalse(any(x.name.endswith(".partial") for x in self.root.iterdir()))

    def test_failed_install_preserves_existing_worker(self):
        (self.root / "apparent-narrator").write_bytes(b"old worker")
        with self.assertRaises(FileNotFoundError): p.install_worker(self.fd, self.root / "missing")
        self.assertEqual((self.root / "apparent-narrator").read_bytes(), b"old worker")
        self.assertFalse(any(x.name.endswith(".partial") for x in self.root.iterdir()))


if __name__ == "__main__":
    unittest.main()
