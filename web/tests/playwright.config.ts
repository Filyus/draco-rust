import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  testMatch: ['browser-smoke.spec.ts', 'splat-render-arms.spec.ts'],
  // Playwright picks the dot reporter on its own once CI is set, which prints
  // neither a test name nor a duration -- so a run that takes four minutes
  // says nothing about which of its eighty tests spent them, and the log of a
  // failure names no test either. `list` is what it already uses locally, so
  // this makes CI say what a developer sees.
  reporter: 'list',
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
