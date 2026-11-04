import importlib.util
from pathlib import Path
import unittest
import tempfile

spec = importlib.util.spec_from_file_location('guard', Path(__file__).parents[1] / 'scripts/check-theme.py')
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

class ThemeGuard(unittest.TestCase):
    def test_owner_builds_check_their_surfaces_and_shared_without_weakening_global_guard(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = ['public/site/src/home.css', 'connect/web/src/app.css',
                     'connect/service/src/recipient/view.css', 'shared/ui/src/extra.css',
                     'capture/desktop/ui/frontend/src/app.css', 'connect/delivery/recipient/tool.ts']
            for path in paths:
                file = root / path
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text('color: #fff;')
            self.assertEqual(set(guard.inventory(root)), set(paths))
            self.assertEqual(set(guard.inventory(root, 'public')), {paths[0], paths[3]})
            self.assertEqual(set(guard.inventory(root, 'connect')), {paths[1], paths[2], paths[3]})
            self.assertEqual(len(guard.violations(guard.inventory(root, 'public'), {})), 2)
            self.assertEqual(len(guard.violations(guard.inventory(root, 'connect'), {})), 3)
            with self.assertRaises(ValueError):
                guard.inventory(root, 'unknown')

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
