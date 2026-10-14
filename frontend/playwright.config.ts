import { defineConfig, devices } from '@playwright/test';

// Flows run against the fake engine in a browser; the real engine is wired
// through Tauri and exercised separately.
export default defineConfig({
  testDir: 'e2e',
  use: { baseURL: 'http://localhost:5173', viewport: { width: 1440, height: 900 } },
  webServer: { command: 'pnpm dev', url: 'http://localhost:5173', reuseExistingServer: true },
  projects: [
    {
      name: 'chromium',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1440, height: 900 },
        // PW_CHANNEL=chrome uses an installed Chrome instead of a downloaded browser.
        ...(process.env.PW_CHANNEL ? { channel: process.env.PW_CHANNEL } : {}),
      },
    },
  ],
});
