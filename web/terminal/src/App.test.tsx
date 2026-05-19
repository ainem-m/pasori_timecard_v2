import { afterEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

type TauriEvent<T> = {
  payload: T
}

const eventMockState = vi.hoisted(() => ({
  cardScannedHandler: null as ((event: TauriEvent<string>) => void | Promise<void>) | null,
}))

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
        event_type: 'clock_in',
        occurred_at: '2026-04-25T09:00:00+09:00[Asia/Tokyo]',
        source: 'nfc',
      }
    }

    return null
  }),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (eventName: string, handler: (event: TauriEvent<string>) => void | Promise<void>) => {
    if (eventName === 'card-scanned') {
      eventMockState.cardScannedHandler = handler
    }

    return () => {}
  }),
}))

import App from './App'

describe('Terminal App', () => {
  afterEach(() => {
    vi.clearAllMocks()
    eventMockState.cardScannedHandler = null
  })

  it('打刻待機画面を表示する', () => {
    render(<App />)

    expect(screen.getByRole('heading', { name: 'カードをタッチ' })).toBeInTheDocument()
  })

  it('時刻同期チェックを10分ごとに予約する', () => {
    const setIntervalSpy = vi.spyOn(globalThis, 'setInterval')

    render(<App />)

    expect(setIntervalSpy).toHaveBeenCalledWith(expect.any(Function), 10 * 60 * 1000)
    setIntervalSpy.mockRestore()
  })

  it('submit_punch 呼び出し時に card_id と event_type のみを渡し、punch_id / occurred_at / source を含めない', async () => {
    vi.mocked(invoke).mockClear()

    await invoke('submit_punch', {
      params: {
        card_id: '0123456789ABCDEF',
        event_type: 'clock_in',
      },
    })

    expect(invoke).toHaveBeenCalledWith('submit_punch', {
      params: {
        card_id: '0123456789ABCDEF',
        event_type: 'clock_in',
      },
    })

    const callArgs = vi.mocked(invoke).mock.calls[0][1] as { params: Record<string, unknown> }
    expect(callArgs.params).not.toHaveProperty('punch_id')
    expect(callArgs.params).not.toHaveProperty('occurred_at')
    expect(callArgs.params).not.toHaveProperty('source')
  })

  it('時刻同期チェックが失敗したら時刻同期エラー画面を表示する', async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === 'check_clock_sync') {
        throw new Error('server unreachable')
      }

      if (command === 'get_reader_status') {
        return 'Ready'
      }

      return null
    })

    render(<App />)

    expect(await screen.findByRole('heading', { name: '時刻同期エラー' })).toBeInTheDocument()
    expect(screen.getByText('時刻同期を確認できません。管理者に連絡してください。')).toBeInTheDocument()
  })

  it('時刻同期エラー中はカードスキャンを無視して打刻を送信しない', async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === 'check_clock_sync') {
        return { is_synced: false, offset_seconds: 11 }
      }

      if (command === 'get_reader_status') {
        return 'Ready'
      }

      if (command === 'resolve_card') {
        return {
          status: 'registered',
          employee: { id: 'emp-1', display_name: '山田 太郎' },
          recent_events: [],
          suggested_type: 'clock_in',
        }
      }

      return null
    })

    render(<App />)

    expect(await screen.findByRole('heading', { name: '時刻同期エラー' })).toBeInTheDocument()
    await waitFor(() => expect(listen).toHaveBeenCalledWith('card-scanned', expect.any(Function)))

    await eventMockState.cardScannedHandler?.({ payload: '0123456789ABCDEF' })

    expect(invoke).not.toHaveBeenCalledWith('resolve_card', expect.anything())
    expect(invoke).not.toHaveBeenCalledWith('submit_punch', expect.anything())
  })

  it('未登録カードではカードIDを隠して従業員選択を表示する', async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === 'check_clock_sync') return { is_synced: true, offset_seconds: 0 }
      if (command === 'get_reader_status') return 'Ready'
      if (command === 'resolve_card') return { status: 'unregistered', card_id: '0123456789ABCDEF' }
      if (command === 'list_active_employees') return [{ id: 'emp-1', display_name: '山田太郎' }]
      return null
    })

    render(<App />)
    await waitFor(() => expect(listen).toHaveBeenCalledWith('card-scanned', expect.any(Function)))

    await eventMockState.cardScannedHandler?.({ payload: '0123456789ABCDEF' })

    expect(await screen.findByRole('heading', { name: '未登録カード' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '山田太郎' })).toBeInTheDocument()
    expect(screen.queryByText(/0123456789ABCDEF/)).not.toBeInTheDocument()
  })

  it('従業員を選んで登録するとカード登録の成功表示を出し打刻は送信しない', async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === 'check_clock_sync') return { is_synced: true, offset_seconds: 0 }
      if (command === 'get_reader_status') return 'Ready'
      if (command === 'resolve_card') return { status: 'unregistered', card_id: '0123456789ABCDEF' }
      if (command === 'list_active_employees') return [{ id: 'emp-1', display_name: '山田太郎' }]
      if (command === 'bind_unregistered_card') {
        return {
          employee: { id: 'emp-1', display_name: '山田太郎' },
          card: { id: 'card-1', employee_id: 'emp-1', card_identifier: '0123456789ABCDEF' },
        }
      }
      return null
    })

    render(<App />)
    await waitFor(() => expect(listen).toHaveBeenCalledWith('card-scanned', expect.any(Function)))
    await eventMockState.cardScannedHandler?.({ payload: '0123456789ABCDEF' })

    fireEvent.click(await screen.findByRole('button', { name: '山田太郎' }))
    fireEvent.click(screen.getByRole('button', { name: '登録' }))

    expect(await screen.findByText('山田太郎に登録しました')).toBeInTheDocument()
    expect(invoke).toHaveBeenCalledWith('bind_unregistered_card', {
      params: {
        card_id: '0123456789ABCDEF',
        employee_id: 'emp-1',
      },
    })
    expect(invoke).not.toHaveBeenCalledWith('submit_punch', expect.anything())
    expect(screen.getByText('カード登録')).toBeInTheDocument()
  })

  it('従業員一覧を取得できない場合は再試行案内を表示する', async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === 'check_clock_sync') return { is_synced: true, offset_seconds: 0 }
      if (command === 'get_reader_status') return 'Ready'
      if (command === 'resolve_card') return { status: 'unregistered', card_id: '0123456789ABCDEF' }
      if (command === 'list_active_employees') throw new Error('offline')
      return null
    })

    render(<App />)
    await waitFor(() => expect(listen).toHaveBeenCalledWith('card-scanned', expect.any(Function)))

    await eventMockState.cardScannedHandler?.({ payload: '0123456789ABCDEF' })

    expect(await screen.findByText('しばらくしてもう一度試してください')).toBeInTheDocument()
  })

  it('カード登録に失敗した場合は再試行案内を表示する', async () => {
    vi.mocked(invoke).mockImplementation(async (command: string) => {
      if (command === 'check_clock_sync') return { is_synced: true, offset_seconds: 0 }
      if (command === 'get_reader_status') return 'Ready'
      if (command === 'resolve_card') return { status: 'unregistered', card_id: '0123456789ABCDEF' }
      if (command === 'list_active_employees') return [{ id: 'emp-1', display_name: '山田太郎' }]
      if (command === 'bind_unregistered_card') throw new Error('conflict')
      return null
    })

    render(<App />)
    await waitFor(() => expect(listen).toHaveBeenCalledWith('card-scanned', expect.any(Function)))
    await eventMockState.cardScannedHandler?.({ payload: '0123456789ABCDEF' })

    fireEvent.click(await screen.findByRole('button', { name: '山田太郎' }))
    fireEvent.click(screen.getByRole('button', { name: '登録' }))

    expect(await screen.findByText('もう一度試してください')).toBeInTheDocument()
  })
})
