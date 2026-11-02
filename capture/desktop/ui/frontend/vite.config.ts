import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { readFileSync } from 'node:fs';

// file: dependencies are snapshots. Refuse a build using stale shared controls.
for (const file of ['button.tsx', 'tabs.tsx', 'utils.ts', 'theme.css']) {
  const canonical = readFileSync(new URL(`../../../../shared/ui/src/${file}`, import.meta.url), 'utf8');
  const installed = readFileSync(new URL(`./node_modules/@apparent/ui/src/${file}`, import.meta.url), 'utf8');
  if (canonical !== installed) throw new Error('Shared UI changed. Run npm ci before building.');
}

export default defineConfig({
  base: './',
  plugins: [react(), tailwindcss()],
});
