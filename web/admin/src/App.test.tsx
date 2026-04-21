import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import App from './App'

const fetchMock = vi.fn()

function okJson(json: unknown) {
  return {
    ok: true,
    status: 200,
    json: async () => json,
  }
}

describe('Admin App', () => {
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
    fetchMock.mockClear()
  })

  it('管理画面の overview を表示する', async () => {
    fetchMock.mockResolvedValue(okJson([]))
    vi.stubGlobal('fetch', fetchMock)

    render(<App />)

    expect(await screen.findByRole('heading', { name: 'Overview' })).toBeInTheDocument()
    expect(fetchMock).toHaveBeenCalled()
  })

  it('未認証時はログイン画面を表示する', async () => {
    fetchMock.mockResolvedValue({
      ok: false,
      status: 401,
      json: async () => [],
    })
    vi.stubGlobal('fetch', fetchMock)

    render(<App />)

    expect(await screen.findByRole('heading', { name: 'Admin Login' })).toBeInTheDocument()
  })

  it('ロック中の管理者ログインでは専用エラーを表示する', async () => {
    fetchMock.mockImplementation(async (input: string) => {
      if (input === '/api/admin/login') {
        return {
          ok: false,
          status: 423,
          json: async () => [],
        }
      }

      return {
        ok: false,
        status: 401,
        json: async () => [],
      }
    })
    vi.stubGlobal('fetch', fetchMock)

    render(<App />)

    await screen.findByRole('heading', { name: 'Admin Login' })
    fireEvent.change(screen.getAllByLabelText('Username')[0], { target: { value: 'admin' } })
    fireEvent.change(screen.getAllByLabelText('Password')[0], { target: { value: 'wrong-password' } })
    fireEvent.click(screen.getByRole('button', { name: 'Sign in' }))

    expect(await screen.findByText('ログイン失敗が続いたため、15分後に再試行してください。')).toBeInTheDocument()
  })

  it('logout 後はログイン画面へ戻る', async () => {
    fetchMock.mockImplementation(async (input: string) => {
      if (input === '/api/admin/logout') {
        return {
          ok: true,
          status: 204,
          json: async () => [],
        }
      }

      return {
        ok: true,
        status: 200,
        json: async () => [],
      }
    })
    vi.stubGlobal('fetch', fetchMock)

    render(<App />)

    const logoutButton = (await screen.findAllByRole('button', { name: 'Logout' }))[0]
    fireEvent.click(logoutButton)

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith('/api/admin/logout', expect.objectContaining({
        method: 'POST',
        credentials: 'same-origin',
      }))
    })
    expect(await screen.findByRole('heading', { name: 'Admin Login' })).toBeInTheDocument()
  })

  it('attendance 画面で月次勤怠を表示する', async () => {
    fetchMock.mockImplementation(async (input: string) => {
      if (input.startsWith('/api/admin/attendance/monthly')) {
        return okJson({
          employee_id: 'emp-1',
          year_month: { year: 2026, month: 4 },
          period_start: '2026-03-16',
          period_end: '2026-04-15',
          total_work_minutes: 1050,
          cutoff_rule: { type: 'day_of_month', day: 15 },
          days: [
            {
              date: '2026-03-16',
              work_minutes: 540,
              has_inconsistency: false,
              status: 'confirmed',
              events: [
                { id: 'p1', employee_id: 'emp-1', event_type: 'clock_in', occurred_at: '2026-03-16T09:00:00+09:00', source: 'nfc' },
                { id: 'p2', employee_id: 'emp-1', event_type: 'clock_out', occurred_at: '2026-03-16T18:00:00+09:00', source: 'nfc' },
              ],
            },
          ],
        })
      }

      if (input === '/api/admin/employees') {
        return okJson([
          {
            id: 'emp-1',
            display_name: '山田太郎',
            employment_type: 'regular',
            affiliation: '受付',
            is_active: true,
            created_at: '2026-04-20T00:00:00+09:00',
          },
        ])
      }

      if (input === '/api/admin/punches' || input === '/api/admin/audit_logs') {
        return okJson([])
      }

      return {
        ok: false,
        status: 404,
        json: async () => [],
      }
    })
    vi.stubGlobal('fetch', fetchMock)

    render(<App />)

    fireEvent.click(await screen.findByRole('button', { name: 'Attendance' }))

    expect(await screen.findByText('2026-03-16 - 2026-04-15')).toBeInTheDocument()
    expect(await screen.findByText('17h 30m')).toBeInTheDocument()
    expect(await screen.findByText('山田太郎')).toBeInTheDocument()

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/admin/attendance/monthly?employee_id=emp-1&year=2026&month=04',
        expect.objectContaining({ credentials: 'same-origin' }),
      )
    })
  })
})
