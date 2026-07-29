import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  testMatch: 'browser-smoke.spec.ts',
  fullyParallel: false,
  workers: 1,
  use: {
    baseURL: 'http://127.0.0.1:4173',
    headless: true,
  },
  webServer: {
    command: 'cargo run --manifest-path dev-server/Cargo.toml -- www 4173',
    cwd: '..',
    url: 'http://127.0.0.1:4173/index.html',
    reuseExistingServer: true,
    timeout: 120000,
  },
});
