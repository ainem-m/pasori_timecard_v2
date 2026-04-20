import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import App from './App'

const fetchMock = vi.fn()

describe('Admin App', () => {
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
    fetchMock.mockClear()
  })

  it('管理画面の overview を表示する', async () => {
    fetchMock.mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => [],
    })
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
})
