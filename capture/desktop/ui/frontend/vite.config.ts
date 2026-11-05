import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { readFileSync } from 'node:fs';

// file: dependencies are snapshots. Refuse a build using stale shared controls.
for (const file of ['button.tsx', 'tabs.tsx', 'utils.ts', 'icons.ts', 'theme.css', 'tokens.css', 'window-theme.cjs', 'status-card.tsx', 'disclosure-menu.tsx', 'status.css', 'feedback/alert.tsx', 'feedback/collapsible.tsx', 'build-identity/index.tsx', 'build-identity/label.ts', 'build-identity/build-identity.css']) {
  const canonical = readFileSync(new URL(`../../../../shared/ui/src/${file}`, import.meta.url), 'utf8');
  const installed = readFileSync(new URL(`./node_modules/@apparent/ui/src/${file}`, import.meta.url), 'utf8');
  if (canonical !== installed) throw new Error('Shared UI changed. Run npm ci before building.');
}

for (const file of [
  'package.json', 'dist/ui/index.js', 'dist/ui/index.d.ts',
  'dist/ui/recording-pill.js', 'dist/ui/recording-pill.d.ts',
  'dist/ui/UploadPocket.js', 'dist/ui/UploadPocket.d.ts',
  'dist/ui/types.d.ts', 'dist/ui/style.css', 'dist/ui/HostGrotesk-Variable.ttf',
]) {
  const canonical = readFileSync(new URL(`../../../../record/${file}`, import.meta.url));
  const installed = readFileSync(new URL(`./node_modules/@apparent/record/${file}`, import.meta.url));
  if (!canonical.equals(installed)) throw new Error('Record package changed. Run npm ci in record, then npm ci here before building.');
}

export default defineConfig(({ mode }) => ({
  base: './',
  plugins: [react(), tailwindcss()],
  build: {
    outDir: mode === 'onboarding-feedback' ? 'node_modules/.cache/onboarding-renderer' : 'dist',
    rollupOptions: { input: mode === 'onboarding-feedback'
      ? 'onboarding.html'
      : { foundation: 'index.html', settings: 'settings.html', onboarding: 'onboarding.html', pill: 'pill/index.html' } },
  },
}));
