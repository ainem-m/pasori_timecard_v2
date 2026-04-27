import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import { invoke } from '@tauri-apps/api/core'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(async (command: string) => {
    if (command === 'check_clock_sync') {
      return { is_synced: true, offset_seconds: 0 }
    }

    if (command === 'get_reader_status') {
      return 'Ready'
    }

    if (command === 'submit_punch') {
      return {
        id: '0192a3b4-c5d6-7e8f-90ab-cdef12345678',
        employee_id: 'emp-1',
        event_type: 'ClockIn',
        occurred_at: '2026-04-25T09:00:00+09:00[Asia/Tokyo]',
        source: 'nfc',
      }
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

  it('submit_punch 呼び出し時に card_id と event_type のみを渡し、punch_id / occurred_at / source を含めない', async () => {
    vi.mocked(invoke).mockClear()

    await invoke('submit_punch', {
      params: {
        card_id: '0123456789ABCDEF',
        event_type: 'ClockIn',
      },
    })

    expect(invoke).toHaveBeenCalledWith('submit_punch', {
      params: {
        card_id: '0123456789ABCDEF',
        event_type: 'ClockIn',
      },
    })

    const callArgs = vi.mocked(invoke).mock.calls[0][1] as { params: Record<string, unknown> }
    expect(callArgs.params).not.toHaveProperty('punch_id')
    expect(callArgs.params).not.toHaveProperty('occurred_at')
    expect(callArgs.params).not.toHaveProperty('source')
  })
})