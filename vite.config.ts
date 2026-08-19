import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { readFileSync } from 'node:fs';

const pkg = JSON.parse(readFileSync(new URL('./package.json', import.meta.url), 'utf-8'));

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [svelte()],
  base: './', // Essential for Tauri webview asset resolution!
  clearScreen: false,
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version || '0.1.0'),
  },
  server: {
    port: 5173,
    strictPort: true,
    host: '127.0.0.1',
  },
  test: {
    environment: 'node',
  },
});
