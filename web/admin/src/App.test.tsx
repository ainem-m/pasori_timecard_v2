import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import App from './App'

const fetchMock = vi.fn()

const employees = [
  {
    id: 'emp-1',
    display_name: '山田太郎',
    employment_type: 'regular',
    affiliation: '受付',
    is_active: true,
    created_at: '2026-04-20T00:00:00+09:00',
  },
]

const punches = [
  {
    id: 'punch-1',
    employee_id: 'emp-1',
    event_type: 'clock_in',
    occurred_at: '2026-04-20T09:00:00+09:00',
    source: 'nfc',
  },
]

const auditLogs = [
  {
    id: 'audit-1',
    actor_type: 'admin',
    action: 'employee.updated',
    target_type: 'employee',
    target_id: 'emp-1',
    created_at: '2026-04-20T10:00:00+09:00',
  },
]

const attendanceRequests = [
  {
    id: 'req-1',
    employee_id: 'emp-1',
    requested_at: '2026-04-20T11:00:00+09:00',
    target_date: '2026-04-19',
    request_type: 'clock_correction',
    reason: '退勤を押し忘れました',
    status: 'pending',
    created_at: '2026-04-20T11:00:00+09:00',
  },
]

function okJson(json: unknown, status = 200) {
  return {
    ok: status >= 200 && status < 300,
    status,
    json: async () => json,
  }
}

function noContent() {
  return {
    ok: true,
    status: 204,
    json: async () => [],
  }
}

function mockAdminApi(overrides: Record<string, unknown> = {}, options: { unauthorizedFirst?: boolean } = {}) {
  const responses: Record<string, unknown> = {
    '/api/admin/employees': employees,
    '/api/admin/punches': punches,
    '/api/admin/audit_logs': auditLogs,
    '/api/admin/attendance_requests': attendanceRequests,
    ...overrides,
  }
  let shouldReturnUnauthorized = options.unauthorizedFirst ?? false

  fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = input.toString()
    const method = init?.method ?? 'GET'

    if (shouldReturnUnauthorized) {
      shouldReturnUnauthorized = false
      return okJson([], 401)
    }

    if (url === '/api/admin/login' && method === 'POST') {
      return noContent()
    }

    if (url === '/api/admin/logout' && method === 'POST') {
      return noContent()
    }

    if (url === '/api/admin/employees' && method === 'POST') {
      return okJson({
        id: 'emp-2',
        display_name: '佐藤花子',
        employment_type: 'part_time',
        affiliation: '診療補助',
        is_active: true,
        created_at: '2026-04-21T00:00:00+09:00',
      })
    }

    if (url === '/api/admin/attendance_requests/req-1/approve' && method === 'POST') {
      return noContent()
    }

    if (url === '/api/admin/attendance_requests/req-1/reject' && method === 'POST') {
      return noContent()
    }

    if (url.startsWith('/api/admin/attendance/monthly')) {
      return okJson({
        employee_id: 'emp-1',
        year_month: { year: 2026, month: 4 },
        period_start: '2026-03-16',
        period_end: '2026-04-15',
        total_work_minutes: 1170,
        policy_profile: 'legacy_regular_2026',
        derived_totals: {
          counted_work_minutes: 1170,
          fixed_time_extra_minutes: 60,
          within_8h_work_minutes: 0,
          over_8h_work_minutes: 0,
          paid_leave_days: 0,
          work_days: 2,
          reference_work_minutes: 1170,
          attendance_notes: [],
        },
        cutoff_rule: { type: 'day_of_month', day: 15 },
        days: [
          {
            date: '2026-03-16',
            work_minutes: 540,
            derived: {
              counted_work_minutes: 540,
              fixed_time_extra_minutes: 0,
              within_8h_work_minutes: 0,
              over_8h_work_minutes: 0,
              paid_leave_days: 0,
              work_days: 1,
              reference_work_minutes: 540,
              attendance_notes: [],
            },
            has_inconsistency: false,
            status: 'confirmed',
            events: [
              {
                id: 'p1',
                employee_id: 'emp-1',
                event_type: 'clock_in',
                occurred_at: '2026-03-16T09:00:00+09:00',
                source: 'nfc',
              },
              {
                id: 'p2',
                employee_id: 'emp-1',
                event_type: 'clock_out',
                occurred_at: '2026-03-16T18:00:00+09:00',
                source: 'nfc',
              },
            ],
          },
          {
            date: '2026-04-15',
            work_minutes: 630,
            derived: {
              counted_work_minutes: 630,
              fixed_time_extra_minutes: 60,
              within_8h_work_minutes: 0,
              over_8h_work_minutes: 0,
              paid_leave_days: 0,
              work_days: 1,
              reference_work_minutes: 630,
              attendance_notes: [],
            },
            has_inconsistency: false,
            status: 'confirmed',
            events: [
              {
                id: 'p3',
                employee_id: 'emp-1',
                event_type: 'clock_in',
                occurred_at: '2026-04-15T09:30:00+09:00',
                source: 'nfc',
              },
              {
                id: 'p4',
                employee_id: 'emp-1',
                event_type: 'clock_out',
                occurred_at: '2026-04-15T20:00:00+09:00',
                source: 'nfc',
              },
            ],
          },
        ],
      })
    }

    if (url.startsWith('/api/admin/attendance_requests')) {
      return okJson(attendanceRequests)
    }

    if (url in responses) {
      return okJson(responses[url])
    }

    return okJson([], 404)
  })

  vi.stubGlobal('fetch', fetchMock)
}

async function login() {
  await screen.findByRole('heading', { name: '管理者ログイン' })
  fireEvent.change(screen.getByLabelText('ユーザー名'), { target: { value: 'admin' } })
  fireEvent.change(screen.getByLabelText('パスワード'), { target: { value: 'password' } })
  fireEvent.click(screen.getByRole('button', { name: 'ログイン' }))
}

describe('Admin App', () => {
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
    fetchMock.mockReset()
  })

  it('ログイン後に従業員一覧を表示する', async () => {
    mockAdminApi({}, { unauthorizedFirst: true })

    render(<App />)

    await login()
    fireEvent.click(await screen.findByRole('button', { name: '従業員' }))

    expect(await screen.findByRole('heading', { name: '従業員管理' })).toBeInTheDocument()
    expect(screen.getAllByText('山田太郎').length).toBeGreaterThan(0)
    expect(screen.getByText('受付')).toBeInTheDocument()
  })

  it('従業員追加 submit が POST /api/admin/employees を呼ぶ', async () => {
    mockAdminApi()

    render(<App />)

    fireEvent.click(await screen.findByRole('button', { name: '従業員' }))
    fireEvent.change(await screen.findByLabelText('氏名'), { target: { value: '佐藤花子' } })
    fireEvent.change(screen.getByLabelText('雇用区分'), { target: { value: 'part_time' } })
    fireEvent.change(screen.getByLabelText('所属'), { target: { value: '診療補助' } })
    fireEvent.change(screen.getByLabelText('備考'), { target: { value: '午前シフト' } })
    fireEvent.click(screen.getByRole('button', { name: '従業員を追加' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/admin/employees',
        expect.objectContaining({
          method: 'POST',
          credentials: 'same-origin',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            display_name: '佐藤花子',
            employment_type: 'part_time',
            affiliation: '診療補助',
            note: '午前シフト',
          }),
        }),
      )
    })
  })

  it('修正申請 approve が /api/admin/attendance_requests/:id/approve を呼ぶ', async () => {
    mockAdminApi()

    render(<App />)

    fireEvent.click(await screen.findByRole('button', { name: '修正申請' }))
    const requestRow = (await screen.findByText('退勤を押し忘れました')).closest('tr')
    expect(requestRow).not.toBeNull()
    fireEvent.click(within(requestRow as HTMLTableRowElement).getByRole('button', { name: '承認' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/admin/attendance_requests/req-1/approve',
        expect.objectContaining({
          method: 'POST',
          credentials: 'same-origin',
        }),
      )
    })
  })

  it('修正申請 reject が /api/admin/attendance_requests/:id/reject を呼ぶ', async () => {
    mockAdminApi()

    render(<App />)

    fireEvent.click(await screen.findByRole('button', { name: '修正申請' }))
    const requestRow = (await screen.findByText('退勤を押し忘れました')).closest('tr')
    expect(requestRow).not.toBeNull()
    fireEvent.click(within(requestRow as HTMLTableRowElement).getByRole('button', { name: '却下' }))

    await waitFor(() => {
      expect(fetchMock).toHaveBeenCalledWith(
        '/api/admin/attendance_requests/req-1/reject',
        expect.objectContaining({
          method: 'POST',
          credentials: 'same-origin',
        }),
      )
    })
  })

  it('監査ログ行が target_id を表示する', async () => {
    mockAdminApi()

    render(<App />)

    fireEvent.click(await screen.findByRole('button', { name: '監査ログ' }))
    const auditRow = (await screen.findByText('employee.updated')).closest('tr')

    expect(auditRow).not.toBeNull()
    expect(within(auditRow as HTMLTableRowElement).getByText('employee:emp-1')).toBeInTheDocument()
  })

  it('月次勤怠に policy profile と補助集計を表示する', async () => {
    mockAdminApi()

    render(<App />)

    fireEvent.click(await screen.findByRole('button', { name: '勤怠' }))

    expect(await screen.findByText('正社員')).toBeInTheDocument()
    expect(screen.getAllByText('残業 1時間 0分').length).toBeGreaterThan(0)
    expect(screen.getByText('2026-04-15')).toBeInTheDocument()
  })
})
