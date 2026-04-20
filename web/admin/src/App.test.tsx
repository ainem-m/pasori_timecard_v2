import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import App from './App'

const fetchMock = vi.fn()

describe('Admin App', () => {
  afterEach(() => {
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
})
