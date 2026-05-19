import { defineConfig, devices } from '@playwright/test'

export default defineConfig({
  testDir: '.',
  testMatch: ['**/*.playwright.ts'],
  timeout: 30_000,
  expect: {
    timeout: 5_000,
  },
  fullyParallel: false,
  retries: 0,
  use: {
    baseURL: 'http://127.0.0.1:5173',
    trace: 'retain-on-failure',
    ...devices['Desktop Chrome'],
  },
  webServer: {
    command: 'pnpm dev -- --host 127.0.0.1',
    url: 'http://127.0.0.1:5173',
    reuseExistingServer: !process.env.CI,
    env: {
      VITE_TERMINAL_E2E: '1',
    },
  },
})
