import { defineConfig } from '@playwright/test';
export default defineConfig({
  testDir: './tests',
  workers: 1,
  use: {
    baseURL: process.env.SAT_DASHBOARD_URL ?? 'http://127.0.0.1:7331',
    browserName: 'chromium',
    viewport: { width: 1440, height: 1100 },
    screenshot: 'only-on-failure',
  },
});
