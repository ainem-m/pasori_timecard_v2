type TauriEvent<T> = {
  payload: T
}

type Listener<T> = (event: TauriEvent<T>) => void | Promise<void>

type PunchType = 'ClockIn' | 'ClockOut'

type InvokeArgs = Record<string, unknown> | undefined

type SubmittedPunch = {
  card_id: string
  event_type: PunchType
}

const scannedCardId = '0123456789ABCDEF'
const submittedPunches: SubmittedPunch[] = []
const listeners = new Map<string, Listener<string>[]>()

function addListener(eventName: string, handler: Listener<string>) {
  const current = listeners.get(eventName) ?? []
  current.push(handler)
  listeners.set(eventName, current)

  return () => {
    const next = (listeners.get(eventName) ?? []).filter((listener) => listener !== handler)
    listeners.set(eventName, next)
  }
}

async function emit(eventName: string, payload: string) {
  for (const handler of listeners.get(eventName) ?? []) {
    await handler({ payload })
  }
}

async function invoke(command: string, args?: InvokeArgs) {
  if (command === 'check_clock_sync') {
    return { is_synced: true, offset_seconds: 0 }
  }

  if (command === 'get_reader_status') {
    return 'Ready'
  }

  if (command === 'resolve_card') {
    return {
      status: 'registered',
      employee: { id: 'emp-terminal-e2e-1', display_name: '山田 太郎' },
      recent_events: [
        {
          event_type: 'ClockOut',
          occurred_at: '2026-04-27T18:10:00+09:00[Asia/Tokyo]',
        },
      ],
      suggested_type: 'ClockIn',
    }
  }

  if (command === 'submit_punch') {
    const params = (args?.params ?? {}) as Partial<SubmittedPunch>
    submittedPunches.push({
      card_id: params.card_id ?? scannedCardId,
      event_type: params.event_type ?? 'ClockIn',
    })

    return {
      id: '0192a3b4-c5d6-7e8f-90ab-cdef12345678',
      employee_id: 'emp-terminal-e2e-1',
      event_type: params.event_type ?? 'ClockIn',
      occurred_at: '2026-04-28T09:00:00+09:00[Asia/Tokyo]',
      source: 'nfc',
    }
  }

  return null
}

export const terminalE2eMocks = {
  invoke,
  listen: async (eventName: string, handler: Listener<string>) => addListener(eventName, handler),
  controls: {
    emitCardScanned: () => emit('card-scanned', scannedCardId),
    submittedPunches: () => [...submittedPunches],
    reset: () => {
      submittedPunches.length = 0
    },
  },
}

declare global {
  interface Window {
    __PASORI_TERMINAL_E2E__?: typeof terminalE2eMocks.controls
  }
}
