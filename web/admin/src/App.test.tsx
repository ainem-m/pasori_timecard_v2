import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import App from './App'

describe('Admin App', () => {
  it('管理画面の見出しを表示する', () => {
    render(<App />)

    expect(
      screen.getByRole('heading', { name: 'PaSoRi Timecard Admin' }),
    ).toBeInTheDocument()
  })
})
