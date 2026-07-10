import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: 1420,
    strictPort: true
  },
  envPrefix: ['VITE_', 'SANSER_'],
  build: {
    target: ['es2022', 'chrome120', 'safari14'],
    sourcemap: false,
    cssCodeSplit: true
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./tests/setup.ts'],
    include: ['src/**/*.test.ts']
  }
});
