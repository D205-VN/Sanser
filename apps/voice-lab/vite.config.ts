import { defineConfig } from 'vitest/config';
export default defineConfig({
  server: { host: '127.0.0.1', port: 1431, strictPort: true, headers: { 'Permissions-Policy': 'camera=(), geolocation=()' } },
  build: { target: 'es2022', sourcemap: false },
  test: { environment: 'jsdom', include: ['src/**/*.test.ts'], restoreMocks: true }
});
