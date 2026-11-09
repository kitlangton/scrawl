import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import ts from 'typescript';

// file: dependencies are snapshots. Refuse a build using stale shared controls.
for (const file of ['button.tsx', 'tabs.tsx', 'utils.ts', 'theme.css', 'tokens.css', 'window-theme.cjs', 'status-card.tsx', 'disclosure-menu.tsx', 'status.css']) {
  const canonical = readFileSync(new URL(`../../../../shared/ui/src/${file}`, import.meta.url), 'utf8');
  const installed = readFileSync(new URL(`./node_modules/@apparent/ui/src/${file}`, import.meta.url), 'utf8');
  if (canonical !== installed) throw new Error('Shared UI changed. Run npm ci before building.');
}

let reportingOutput: string;
export default defineConfig({
  base: './',
  plugins: [react(), tailwindcss(), {
    name: 'desktop-reporting-policy',
    apply: 'build',
    configResolved(config) { reportingOutput = path.resolve(config.root, config.build.outDir, config.build.assetsDir); },
    // Emitted beside other staged assets; never commit authored JavaScript.
    closeBundle() {
      const source = readFileSync(new URL('./electron/reporting/index.ts', import.meta.url), 'utf8');
      const { outputText } = ts.transpileModule(source, {
        compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
        fileName: 'index.ts',
      });
      mkdirSync(reportingOutput, { recursive: true });
      writeFileSync(path.join(reportingOutput, 'reporting.cjs'), outputText);
    },
  }],
  build: { rollupOptions: { input: { foundation: 'index.html', settings: 'settings.html', onboarding: 'onboarding.html', installer: 'src/installer/index.html' } } },
});
