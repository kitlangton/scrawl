"""Keep legacy Python test discovery connected to the authoritative TS guard suite."""
from pathlib import Path
import subprocess
import unittest


class ThemeGuard(unittest.TestCase):
    def test_typescript_guard_contract(self):
        subprocess.run(['node', '--test', str(Path(__file__).with_name('guard.test.ts'))], check=True)


if __name__ == '__main__':
    unittest.main()
