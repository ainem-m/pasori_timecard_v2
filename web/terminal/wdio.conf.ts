import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process'
import path from 'node:path'
import process from 'node:process'
import { browser } from '@wdio/globals'
import type { Options } from '@wdio/types'

let tauriDriver: ChildProcessWithoutNullStreams | undefined

export const config: Options.Testrunner = {
  runner: 'local',
  specs: ['./e2e/**/*.e2e.ts'],
  maxInstances: 1,
  hostname: '127.0.0.1',
  port: 4444,
  path: '/',
  logLevel: 'info',
  framework: 'mocha',
  reporters: ['spec'],
  mochaOpts: {
    ui: 'bdd',
    timeout: 60_000,
  },
  capabilities: [
    {
      browserName: 'wry',
      'tauri:options': {
        application: path.resolve('../../target/debug/terminal'),
      },
    },
  ],
  autoCompileOpts: {
    autoCompile: true,
    tsNodeOpts: {
      transpileOnly: true,
      project: './tsconfig.node.json',
    },
  },
  onPrepare: async () => {
    if (process.platform === 'darwin') {
      throw new Error(
        'Official tauri-driver desktop WebDriver is not supported on macOS. Use `pnpm -C web/terminal test:e2e` for local UI E2E, and run `test:e2e:tauri` on Linux or Windows.',
      )
    }

    tauriDriver = spawn('tauri-driver', [], {
      stdio: 'pipe',
      env: {
        ...process.env,
        VITE_TERMINAL_E2E: '1',
      },
    })

    await new Promise((resolve) => setTimeout(resolve, 1_000))
  },
  before: async () => {
    await browser.setTimeout({ implicit: 2_000 })
  },
  onComplete: () => {
    tauriDriver?.kill()
    tauriDriver = undefined
  },
}
