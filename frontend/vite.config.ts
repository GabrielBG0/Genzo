import { fileURLToPath, URL } from 'node:url';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';

const contract = fileURLToPath(new URL('../contract', import.meta.url));

export default defineConfig({
  plugins: [svelte()],
  resolve: {
    alias: {
      $contract: contract,
      $lib: fileURLToPath(new URL('./src/lib', import.meta.url)),
    },
    // Component tests mount Svelte's browser build, not its server build.
    ...(process.env.VITEST ? { conditions: ['browser'] } : {}),
  },
  // Tauri expects a fixed port and its own console output.
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
    fs: { allow: ['.', contract] },
  },
  build: {
    target: 'es2022',
  },
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.ts'],
    setupFiles: ['src/test-setup.ts'],
  },
});
