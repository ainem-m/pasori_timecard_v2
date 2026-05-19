import { $, browser, expect } from '@wdio/globals'

type SubmittedPunch = {
  card_id: string
  event_type: 'clock_in' | 'clock_out'
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

describe('打刻端末 mocked scan happy path', () => {
  it('カードスキャンから確認画面を表示し、長押し確定で打刻を送信する', async () => {
    await browser.waitUntil(
      async () => browser.execute(() => Boolean(window.__PASORI_TERMINAL_E2E__)),
      { timeout: 10_000, timeoutMsg: 'E2E mock controls were not registered' },
    )

    await browser.execute(() => window.__PASORI_TERMINAL_E2E__?.reset())
    await expect($('h2=カードをタッチ')).toBeDisplayed()

    await browser.executeAsync((done) => {
      window.__PASORI_TERMINAL_E2E__?.emitCardScanned().then(() => done())
    })

    await expect($('h2*=山田 太郎')).toBeDisplayed()
    await expect($('div=出勤')).toBeDisplayed()

    const confirmButton = await $('button=CONFIRM')
    await confirmButton.moveTo()
    await browser.performActions([
      {
        type: 'pointer',
        id: 'mouse',
        parameters: { pointerType: 'mouse' },
        actions: [
          { type: 'pointerMove', origin: await confirmButton, x: 0, y: 0 },
          { type: 'pointerDown', button: 0 },
          { type: 'pause', duration: 1_100 },
          { type: 'pointerUp', button: 0 },
        ],
      },
    ])
    await browser.releaseActions()

    await expect($('h2=DONE!')).toBeDisplayed()
    await expect($('p*=山田 太郎 さん、おはようございます')).toBeDisplayed()

    const submittedPunches = await browser.execute(() => (
      window.__PASORI_TERMINAL_E2E__?.submittedPunches() ?? []
    ))

    expect(submittedPunches).toEqual([
      {
        card_id: '0123456789ABCDEF',
        event_type: 'clock_in',
      },
    ])
  })
})
