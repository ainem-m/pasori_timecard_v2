import { expect, test } from '@playwright/test'

type SubmittedPunch = {
  card_id: string
  event_type: 'ClockIn' | 'ClockOut'
}

declare global {
  interface Window {
    __PASORI_TERMINAL_E2E__?: {
      emitCardScanned: () => Promise<void>
      submittedPunches: () => SubmittedPunch[]
      reset: () => void
    }
  }
}

test.describe('打刻端末 UI happy path', () => {
  test('カードスキャンから確認画面を表示し、長押し確定で打刻を送信する', async ({ page }) => {
    await page.goto('/')

    await expect(page.getByRole('heading', { name: 'カードをタッチ' })).toBeVisible()
    await page.waitForFunction(() => Boolean(window.__PASORI_TERMINAL_E2E__))

    await page.evaluate(() => window.__PASORI_TERMINAL_E2E__?.reset())
    await page.evaluate(async () => window.__PASORI_TERMINAL_E2E__?.emitCardScanned())

    await expect(page.getByRole('heading', { name: /山田 太郎/ })).toBeVisible()
    await expect(page.getByText('出勤').first()).toBeVisible()

    const confirmButton = page.getByRole('button', { name: 'CONFIRM' })
    await confirmButton.hover()
    await page.mouse.down()
    await page.waitForTimeout(1_100)
    await page.mouse.up()

    await expect(page.getByRole('heading', { name: 'DONE!' })).toBeVisible()
    await expect(page.getByText('山田 太郎 さん、おはようございます')).toBeVisible()

    const submittedPunches = await page.evaluate(() => (
      window.__PASORI_TERMINAL_E2E__?.submittedPunches() ?? []
    ))

    expect(submittedPunches).toEqual([
      {
        card_id: '0123456789ABCDEF',
        event_type: 'ClockIn',
      },
    ])
  })
})
