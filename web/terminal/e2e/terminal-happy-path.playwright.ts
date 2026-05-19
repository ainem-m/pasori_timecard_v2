import { expect, test } from '@playwright/test'

type SubmittedPunch = {
  card_id: string
  event_type: 'clock_in' | 'clock_out'
}

type BoundCard = {
  card_id: string
  employee_id: string
}

declare global {
  interface Window {
    __PASORI_TERMINAL_E2E__?: {
      emitCardScanned: () => Promise<void>
      emitUnregisteredCardScanned: () => Promise<void>
      submittedPunches: () => SubmittedPunch[]
      boundCards: () => BoundCard[]
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
        event_type: 'clock_in',
      },
    ])
  })

  test('未登録カードから従業員を選択してカード登録し、打刻は送信しない', async ({ page }) => {
    await page.goto('/')

    await expect(page.getByRole('heading', { name: 'カードをタッチ' })).toBeVisible()
    await page.waitForFunction(() => Boolean(window.__PASORI_TERMINAL_E2E__))

    await page.evaluate(() => window.__PASORI_TERMINAL_E2E__?.reset())
    await page.evaluate(async () => window.__PASORI_TERMINAL_E2E__?.emitUnregisteredCardScanned())

    await expect(page.getByRole('heading', { name: '未登録カード' })).toBeVisible()
    await expect(page.getByRole('button', { name: '山田 太郎' })).toBeVisible()
    await expect(page.getByText('0123456789ABCDEF')).toHaveCount(0)

    await page.getByRole('button', { name: '山田 太郎' }).click()
    await expect(page.getByRole('heading', { name: 'カード登録' })).toBeVisible()
    await page.getByRole('button', { name: '登録' }).click()

    await expect(page.getByText('山田 太郎に登録しました')).toBeVisible()
    await expect(page.getByText('カード登録')).toBeVisible()

    const result = await page.evaluate(() => ({
      submittedPunches: window.__PASORI_TERMINAL_E2E__?.submittedPunches() ?? [],
      boundCards: window.__PASORI_TERMINAL_E2E__?.boundCards() ?? [],
    }))

    expect(result.submittedPunches).toEqual([])
    expect(result.boundCards).toEqual([
      {
        card_id: '0123456789ABCDEF',
        employee_id: 'emp-terminal-e2e-1',
      },
    ])
  })
})
