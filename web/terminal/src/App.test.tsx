import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import App from './App'

describe('Terminal App', () => {
  it('打刻端末の見出しを表示する', () => {
    render(<App />)

    expect(
      screen.getByRole('heading', { name: 'PaSoRi Timecard Terminal' }),
    ).toBeInTheDocument()
  })
})
