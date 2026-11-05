import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';
const root = new URL('../', import.meta.url);
// Publish compiled shared controls inside the UI artifact. React remains host-owned.
await build({
  absWorkingDir: fileURLToPath(root), entryPoints: ['src/ui/index.ts'],
  outfile: 'dist/ui/index.js', bundle: true, format: 'esm', platform: 'browser', preserveSymlinks: true,
  jsx: 'automatic', external: ['react', 'react/*', 'react-dom', 'react-dom/*'],
  loader: { '.css': 'empty' },
});
