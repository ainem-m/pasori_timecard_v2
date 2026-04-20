import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (command: string) => {
    if (command === 'check_clock_sync') {
      return { is_synced: true, offset_seconds: 0 }
    }

    if (command === 'get_reader_status') {
      return 'Ready'
    }

    return null
  }),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}))

import App from './App'

describe('Terminal App', () => {
  afterEach(() => {
    vi.clearAllMocks()
  })

  it('打刻待機画面を表示する', () => {
    render(<App />)

    expect(screen.getByRole('heading', { name: 'カードをタッチ' })).toBeInTheDocument()
  })
})
