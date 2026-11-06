import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { checkDependencyFreshness } from './scripts/check-dependency-freshness.ts';

// file: dependencies are snapshots. Refuse a build using stale shared controls.
checkDependencyFreshness();

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
