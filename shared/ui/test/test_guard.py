import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('guard', Path(__file__).parents[1] / 'scripts/check-theme.py')
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

class ThemeGuard(unittest.TestCase):
    def test_public_scope_includes_shared_colors_but_does_not_read_other_owners(self):
        for git in (False, True):
            with self.subTest(git=git), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                if git:
                    subprocess.run(['git', 'init', '-q', directory], check=True)
                for owner in ('public', 'shared', 'connect', 'capture'):
                    path = root / owner / 'src/new.css'
                    path.parent.mkdir(parents=True)
                    path.write_text('color:#abcdef;')
                scoped = guard.inventory(root, owner='public')
                self.assertEqual(set(scoped), {'public/src/new.css', 'shared/src/new.css'})
                self.assertEqual(len(guard.violations(scoped, {})), 2)
                self.assertEqual(len(guard.inventory(root)), 4)

    def test_rejects_fixed_colors_in_new_files_and_growth_in_legacy_files(self):
        baseline = {'public/old.css': {'#fff': 1}}
        self.assertEqual(guard.violations({'public/old.css': {'#fff': 1}}, baseline), [])
        self.assertEqual(len(guard.violations({'public/new.css': {'#fff': 1}, 'public/old.css': {'#fff': 2}}, baseline)), 2)

    def test_detects_common_css_and_tailwind_bypasses(self):
        for value in ['color:#fff;', 'background:rgb(0 0 0 / .5)', 'bg-white', 'text-zinc-950', 'dark:bg-black', 'color-scheme: light;', 'background:white;color:black', 'backgroundColor: "red"']:
            self.assertTrue(guard.colors_in(value), value)
        for value in ['bg-background', 'text-foreground', 'var(--color-surface)', 'color:currentColor']:
            self.assertFalse(guard.colors_in(value), value)

if __name__ == '__main__':
    unittest.main()
