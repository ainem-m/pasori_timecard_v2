import { afterEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@testing-library/react'
import App from './App'

const fetchMock = vi.fn(async () => ({
  ok: true,
  json: async () => [],
}))

describe('Admin App', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
    fetchMock.mockClear()
  })

  it('管理画面の overview を表示する', async () => {
    vi.stubGlobal('fetch', fetchMock)

    render(<App />)

    expect(await screen.findByRole('heading', { name: 'Overview' })).toBeInTheDocument()
    expect(fetchMock).toHaveBeenCalled()
  })
})
