import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('guard', Path(__file__).parents[1] / 'scripts/check-theme.py')
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

class ThemeGuard(unittest.TestCase):
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
