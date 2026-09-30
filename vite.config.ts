import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
const projectRoot = fileURLToPath(new URL('.', import.meta.url));
const projectPath = (relative: string) => resolve(projectRoot, relative);
export default defineConfig({
  root: projectPath('apps/desktop'), publicDir: projectPath('assets'), plugins: [react()],
  server: { host: '127.0.0.1', port: 1420, strictPort: true, fs: { strict: true, allow: ['apps/desktop', 'packages/cushion-sdk/src', 'tests/fixtures', 'assets', 'node_modules'].map(projectPath) } },
  build: { target: 'es2022', outDir: 'dist', sourcemap: false, rollupOptions: { input: { main: projectPath('apps/desktop/index.html'), embedded: projectPath('apps/desktop/embedded.html'), support: projectPath('apps/desktop/support.html') } } },
});
