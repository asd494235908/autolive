import { readFileSync } from 'node:fs';
import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

const packageJson = JSON.parse(
  readFileSync(new URL('./package.json', import.meta.url), 'utf8'),
) as { version?: string };

export default defineConfig({
  plugins: [react()],
  // Tauri serves the bundled frontend from its asset protocol, not `/`.
  base: './',
  clearScreen: false,
  define: {
    __APP_VERSION__: JSON.stringify(packageJson.version ?? '0.1.0'),
  },
});
