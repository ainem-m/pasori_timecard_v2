import { type FormEvent, useEffect, useState } from 'react';
import { 
  Users, 
  Clock, 
  ShieldAlert, 
  LayoutDashboard, 
  Plus, 
  ChevronRight,
  UserPlus,
  ClipboardCheck,
  CreditCard,
  UserMinus,
  type LucideIcon,
} from 'lucide-react';
import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';
import { format } from 'date-fns';

/** Utility for tailwind classes */
function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

// --- Types ---

interface Employee {
  id: string;
  display_name: string;
  employment_type: string;
  affiliation?: string;
  note?: string;
  is_active: boolean;
  created_at: string;
}

interface PunchEvent {
  id: string;
  employee_id: string;
  event_type: 'clock_in' | 'clock_out' | 'break_start' | 'break_end' | 'temporary_out' | 'temporary_return' | 'manual_correction';
  occurred_at: string;
  source: string;
}

interface AuditLog {
  id: string;
  actor_type: string;
  action: string;
  target_type: string;
  target_id?: string;
  created_at: string;
}

interface LoginFormState {
  username: string;
  password: string;
}

interface AttendanceDay {
  date: string;
  events: PunchEvent[];
  work_minutes: number;
  has_inconsistency: boolean;
  status: 'unconfirmed' | 'confirmed' | 'locked';
}

interface MonthlyAttendance {
  employee_id: string;
  year_month: {
    year: number;
    month: number;
  };
  days: AttendanceDay[];
  total_work_minutes: number;
  cutoff_rule:
    | {
        type: 'day_of_month';
        day: number;
      }
    | {
        type: 'end_of_month';
      };
  period_start: string;
  period_end: string;
}

interface EmployeeFormState {
  display_name: string;
  employment_type: string;
  affiliation: string;
  note: string;
}

interface CardBindFormState {
  card_identifier: string;
  employee_id: string;
}

interface AttendanceRequest {
  id: string;
  employee_id: string;
  requested_payload_json?: string;
  requested_at?: string;
  target_date?: string;
  request_type?: string;
  reason?: string;
  review_note?: string;
  status: string;
  created_at: string;
}

function formatPunchEventLabel(eventType: PunchEvent['event_type']) {
  switch (eventType) {
    case 'clock_in':
      return '出勤';
    case 'clock_out':
      return '退勤';
    case 'break_start':
      return '休憩開始';
    case 'break_end':
      return '休憩終了';
    case 'temporary_out':
      return '一時外出';
    case 'temporary_return':
      return '戻り';
    case 'manual_correction':
      return '修正';
  }
}

function formatMinutes(totalMinutes: number) {
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return `${hours}時間 ${minutes}分`;
}

function buildCurrentYearMonth() {
  return format(new Date(), 'yyyy-MM');
}

function formatAttendanceStatus(status: AttendanceDay['status']) {
  switch (status) {
    case 'unconfirmed':
      return '未確認';
    case 'confirmed':
      return '確認済み';
    case 'locked':
      return '締め済み';
  }
}

function formatAttendanceRequest(request: AttendanceRequest) {
  let payload: Record<string, unknown> = {};
  if (request.requested_payload_json) {
    try {
      const parsed = JSON.parse(request.requested_payload_json);
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        payload = parsed as Record<string, unknown>;
      }
    } catch {
      payload = {};
    }
  }

  const payloadDate = typeof payload.date === 'string' ? payload.date : undefined;
  const payloadReason = typeof payload.reason === 'string' ? payload.reason : undefined;
  const payloadTime = typeof payload.time === 'string' ? payload.time : undefined;
  const payloadTarget = typeof payload.target === 'string' ? payload.target : undefined;

  return {
    targetDate: request.target_date || payloadDate || '-',
    title: request.request_type || '打刻修正',
    detail: request.reason || payloadReason || [payloadTarget, payloadTime].filter(Boolean).join(' ') || '理由は未入力です。',
  };
}

// --- Components ---

const SidebarItem = ({ icon: Icon, label, active, onClick }: { icon: LucideIcon, label: string, active?: boolean, onClick: () => void }) => (
  <button 
    onClick={onClick}
    className={cn(
      "flex items-center gap-3 w-full px-4 py-3 rounded-lg transition-all duration-200 group",
      active 
        ? "bg-primary/10 text-primary font-medium" 
        : "text-muted-foreground hover:bg-accent hover:text-accent-foreground"
    )}
  >
    <Icon className={cn("w-5 stealth-5 transition-transform group-hover:scale-110", active && "text-primary")} />
    <span>{label}</span>
    {active && <ChevronRight className="ml-auto w-4 h-4" />}
  </button>
);

const StatCard = ({ title, value, icon: Icon, trend }: { title: string, value: string | number, icon: LucideIcon, trend?: string }) => (
  <div className="card p-6 flex items-start justify-between">
    <div>
      <p className="text-sm font-medium text-muted-foreground mb-1">{title}</p>
      <h3 className="text-3xl font-bold tracking-tight">{value}</h3>
      {trend && <p className="text-xs text-green-500 mt-2 font-medium">{trend}</p>}
    </div>
    <div className="p-3 bg-accent rounded-lg">
      <Icon className="w-6 h-6 text-accent-foreground" />
    </div>
  </div>
);

// --- Main App ---

export default function App() {
  const [view, setView] = useState<'dashboard' | 'employees' | 'attendance' | 'requests' | 'audit'>('dashboard');
  const [employees, setEmployees] = useState<Employee[]>([]);
  const [punches, setPunches] = useState<PunchEvent[]>([]);
  const [auditLogs, setAuditLogs] = useState<AuditLog[]>([]);
  const [attendanceRequests, setAttendanceRequests] = useState<AttendanceRequest[]>([]);
  const [monthlyAttendance, setMonthlyAttendance] = useState<MonthlyAttendance | null>(null);
  const [isLoading, setLoading] = useState(true);
  const [isAttendanceLoading, setAttendanceLoading] = useState(false);
  const [isRequestLoading, setRequestLoading] = useState(false);
  const [needsLogin, setNeedsLogin] = useState(false);
  const [loginError, setLoginError] = useState<string | null>(null);
  const [loginForm, setLoginForm] = useState<LoginFormState>({ username: '', password: '' });
  const [employeeForm, setEmployeeForm] = useState<EmployeeFormState>({
    display_name: '',
    employment_type: '',
    affiliation: '',
    note: '',
  });
  const [cardBindForm, setCardBindForm] = useState<CardBindFormState>({
    card_identifier: '',
    employee_id: '',
  });
  const [reviewNotes, setReviewNotes] = useState<Record<string, string>>({});
  const [selectedEmployeeId, setSelectedEmployeeId] = useState('');
  const [selectedYearMonth, setSelectedYearMonth] = useState(buildCurrentYearMonth);

  async function fetchData() {
    setLoading(true);
    try {
      const [empRes, punchRes, auditRes] = await Promise.all([
        fetch('/api/admin/employees', { credentials: 'same-origin' }),
        fetch('/api/admin/punches', { credentials: 'same-origin' }),
        fetch('/api/admin/audit_logs', { credentials: 'same-origin' })
      ]);

      const responses = [empRes, punchRes, auditRes];
      if (responses.some((response) => response.status === 401)) {
        setNeedsLogin(true);
        return;
      }

      if (empRes.ok) setEmployees(await empRes.json());
      if (punchRes.ok) setPunches(await punchRes.json());
      if (auditRes.ok) setAuditLogs(await auditRes.json());
      setNeedsLogin(false);
      setLoginError(null);
    } catch (err) {
      console.error('Failed to fetch data', err);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    fetchData();
  }, [view]);

  useEffect(() => {
    if (employees.length === 0) {
      return;
    }

    const selectedEmployeeStillExists = employees.some((employee) => employee.id === selectedEmployeeId);
    if (!selectedEmployeeStillExists) {
      setSelectedEmployeeId(employees[0].id);
    }

    const bindEmployeeStillExists = employees.some((employee) => employee.id === cardBindForm.employee_id);
    if (!bindEmployeeStillExists) {
      setCardBindForm((current) => ({ ...current, employee_id: employees[0].id }));
    }
  }, [cardBindForm.employee_id, employees, selectedEmployeeId]);

  useEffect(() => {
    async function fetchMonthlyAttendance() {
      if (view !== 'attendance' || !selectedEmployeeId) {
        return;
      }

      const [year, month] = selectedYearMonth.split('-');
      if (!year || !month) {
        return;
      }

      setAttendanceLoading(true);
      try {
        const response = await fetch(
          `/api/admin/attendance/monthly?employee_id=${selectedEmployeeId}&year=${year}&month=${month}`,
          { credentials: 'same-origin' },
        );

        if (response.status === 401) {
          setNeedsLogin(true);
          return;
        }

        if (!response.ok) {
          setMonthlyAttendance(null);
          return;
        }

        setMonthlyAttendance(await response.json());
      } catch (err) {
        console.error('Failed to fetch monthly attendance', err);
        setMonthlyAttendance(null);
      } finally {
        setAttendanceLoading(false);
      }
    }

    fetchMonthlyAttendance();
  }, [view, selectedEmployeeId, selectedYearMonth]);

  async function fetchAttendanceRequests() {
    setRequestLoading(true);
    try {
      const response = await fetch('/api/admin/attendance_requests?status=requested', {
        credentials: 'same-origin',
      });

      if (response.status === 401) {
        setNeedsLogin(true);
        return;
      }

      if (response.ok) {
        setAttendanceRequests(await response.json());
      }
    } catch (err) {
      console.error('Failed to fetch attendance requests', err);
    } finally {
      setRequestLoading(false);
    }
  }

  useEffect(() => {
    if (view === 'requests') {
      fetchAttendanceRequests();
    }
  }, [view]);

  async function handleLoginSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setLoginError(null);
    setLoading(true);

    try {
      const response = await fetch('/api/admin/login', {
        method: 'POST',
        credentials: 'same-origin',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify(loginForm),
      });

      if (response.status === 401) {
        setLoginError('ユーザー名またはパスワードが正しくありません。');
        return;
      }

      if (response.status === 423) {
        setLoginError('ログイン失敗が続いたため、15分後に再試行してください。');
        return;
      }

      if (!response.ok) {
        setLoginError('ログインに失敗しました。時間をおいて再試行してください。');
        return;
      }

      setNeedsLogin(false);
      setLoginForm({ username: '', password: '' });
      await fetchData();
    } catch (err) {
      console.error('Failed to login', err);
      setLoginError('ログインに失敗しました。時間をおいて再試行してください。');
    } finally {
      setLoading(false);
    }
  }

  async function handleEmployeeSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();

    const response = await fetch('/api/admin/employees', {
      method: 'POST',
      credentials: 'same-origin',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(employeeForm),
    });

    if (response.status === 401) {
      setNeedsLogin(true);
      return;
    }

    if (response.ok) {
      setEmployeeForm({ display_name: '', employment_type: '', affiliation: '', note: '' });
      await fetchData();
    }
  }

  async function handleDeactivateEmployee(employeeId: string) {
    const response = await fetch(`/api/admin/employees/${employeeId}`, {
      method: 'DELETE',
      credentials: 'same-origin',
    });

    if (response.status === 401) {
      setNeedsLogin(true);
      return;
    }

    if (response.ok) {
      await fetchData();
    }
  }

  async function handleCardBindSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();

    const response = await fetch('/api/admin/cards/bind', {
      method: 'POST',
      credentials: 'same-origin',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(cardBindForm),
    });

    if (response.status === 401) {
      setNeedsLogin(true);
      return;
    }

    if (response.ok) {
      setCardBindForm((current) => ({ ...current, card_identifier: '' }));
      await fetchData();
    }
  }

  async function handleReviewAttendanceRequest(requestId: string, decision: 'approve' | 'reject') {
    const response = await fetch(`/api/admin/attendance_requests/${requestId}/${decision}`, {
      method: 'POST',
      credentials: 'same-origin',
      headers: {
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ review_note: reviewNotes[requestId] || undefined }),
    });

    if (response.status === 401) {
      setNeedsLogin(true);
      return;
    }

    if (response.ok) {
      setReviewNotes((current) => {
        const next = { ...current };
        delete next[requestId];
        return next;
      });
      await fetchAttendanceRequests();
      await fetchData();
    }
  }

  async function handleLogout() {
    setLoading(true);
    try {
      await fetch('/api/admin/logout', {
        method: 'POST',
        credentials: 'same-origin',
      });
    } catch (err) {
      console.error('Failed to logout', err);
    } finally {
      setEmployees([]);
      setPunches([]);
      setAuditLogs([]);
      setAttendanceRequests([]);
      setMonthlyAttendance(null);
      setNeedsLogin(true);
      setLoginError(null);
      setLoading(false);
    }
  }

  if (needsLogin) {
    return (
      <div className="min-h-screen bg-background text-foreground flex items-center justify-center p-6">
        <div className="w-full max-w-md card p-8 space-y-6">
          <div className="space-y-2 text-center">
            <h1 className="text-3xl font-bold tracking-tight">管理者ログイン</h1>
            <p className="text-sm text-muted-foreground">管理画面を利用するにはログインが必要です。</p>
          </div>

          <form className="space-y-4" onSubmit={handleLoginSubmit}>
            <label className="block space-y-2">
              <span className="text-sm font-medium">ユーザー名</span>
              <input
                className="w-full rounded-lg border border-border bg-background px-3 py-2"
                value={loginForm.username}
                onChange={(event) => setLoginForm((current) => ({ ...current, username: event.target.value }))}
                autoComplete="username"
              />
            </label>
            <label className="block space-y-2">
              <span className="text-sm font-medium">パスワード</span>
              <input
                type="password"
                className="w-full rounded-lg border border-border bg-background px-3 py-2"
                value={loginForm.password}
                onChange={(event) => setLoginForm((current) => ({ ...current, password: event.target.value }))}
                autoComplete="current-password"
              />
            </label>

            {loginError && (
              <p className="text-sm text-red-500">{loginError}</p>
            )}

            <button
              type="submit"
              className="w-full rounded-lg bg-brand text-brand-foreground py-2 font-medium disabled:opacity-60"
              disabled={isLoading}
            >
              {isLoading ? 'ログイン中...' : 'ログイン'}
            </button>
          </form>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen flex bg-background">
      {/* Sidebar */}
      <aside className="w-64 border-r border-border flex flex-col p-4 gap-6 shrink-0 bg-muted/30 backdrop-blur-xl">
        <div className="px-2 flex items-center gap-2 mb-2">
          <div className="w-8 h-8 bg-brand rounded-lg flex items-center justify-center shadow-lg shadow-brand/20">
            <Clock className="w-5 h-5 text-brand-foreground" />
          </div>
          <span className="font-bold text-xl tracking-tight">Timecard <span className="text-brand">v2</span></span>
        </div>

        <nav className="flex flex-col gap-1">
          <SidebarItem icon={LayoutDashboard} label="ダッシュボード" active={view === 'dashboard'} onClick={() => setView('dashboard')} />
          <SidebarItem icon={Users} label="従業員" active={view === 'employees'} onClick={() => setView('employees')} />
          <SidebarItem icon={Clock} label="勤怠" active={view === 'attendance'} onClick={() => setView('attendance')} />
          <SidebarItem icon={ClipboardCheck} label="修正申請" active={view === 'requests'} onClick={() => setView('requests')} />
          <SidebarItem icon={ShieldAlert} label="監査ログ" active={view === 'audit'} onClick={() => setView('audit')} />
        </nav>
      </aside>

      {/* Main Content */}
      <main className="flex-1 flex flex-col min-w-0 overflow-y-auto">
        {/* Header */}
        <header className="h-16 border-b border-border flex items-center justify-between px-8 bg-background/80 backdrop-blur-md sticky top-0 z-10">
          <h1 className="text-lg font-semibold capitalize">
            {view === 'dashboard' && 'ダッシュボード'}
            {view === 'employees' && '従業員'}
            {view === 'attendance' && '勤怠'}
            {view === 'requests' && '修正申請'}
            {view === 'audit' && '監査ログ'}
          </h1>
          
          <div className="flex items-center gap-4">
            <button
              type="button"
              onClick={handleLogout}
              className="rounded-full border border-border px-4 py-2 text-sm font-medium transition-colors hover:bg-accent"
            >
              ログアウト
            </button>
            <div className="w-8 h-8 rounded-full bg-accent border border-border"></div>
          </div>
        </header>

        {/* Content Area */}
        <div className="p-8">
          {view === 'dashboard' && (
            <div className="space-y-8 animate-in fade-in slide-in-from-bottom-2 duration-500">
              <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
                <StatCard title="従業員数" value={employees.length} icon={Users} />
                <StatCard title="本日の打刻者" value={new Set(punches.map(p => p.employee_id)).size} icon={UserPlus} />
                <StatCard title="打刻件数" value={punches.length} icon={Clock} />
                <StatCard title="システム状態" value="稼働中" icon={ShieldAlert} />
              </div>

              <div className="grid grid-cols-1 lg:grid-cols-3 gap-8">
                <div className="lg:col-span-2 space-y-4">
                  <div className="flex items-center justify-between">
                    <h2 className="text-xl font-bold">最近の打刻</h2>
                    <button onClick={() => setView('attendance')} className="text-sm text-brand font-medium hover:underline">勤怠を見る</button>
                  </div>
                  <div className="card divide-y divide-border">
                    {punches.slice(0, 5).map(punch => (
                      <div key={punch.id} className="p-4 flex items-center justify-between hover:bg-accent/30 transition-colors">
                        <div className="flex items-center gap-3">
                          <div className={cn(
                            "w-2 h-2 rounded-full",
                            punch.event_type === 'clock_in' ? "bg-green-500" : "bg-red-500"
                          )} />
                          <div>
                            <p className="font-medium text-sm">
                              {employees.find(e => e.id === punch.employee_id)?.display_name || '不明な従業員'}
                            </p>
                            <p className="text-xs text-muted-foreground">{formatPunchEventLabel(punch.event_type)}</p>
                          </div>
                      </div>
                      <p className="text-sm text-muted-foreground">{format(new Date(punch.occurred_at), 'HH:mm:ss')}</p>
                    </div>
                    ))}
                    {punches.length === 0 && <p className="p-8 text-center text-muted-foreground italic">打刻はまだありません。</p>}
                  </div>
                </div>

                <div className="space-y-4">
                  <h2 className="text-xl font-bold">最近の操作</h2>
                  <div className="card p-4 space-y-6">
                    {auditLogs.slice(0, 5).map(log => (
                      <div key={log.id} className="flex gap-4 relative">
                        <div className="w-px h-full bg-border absolute left-2.5 top-5"></div>
                        <div className="w-5 h-5 rounded-full bg-accent border border-border flex items-center justify-center shrink-0 z-0 bg-background">
                          <ShieldAlert className="w-3 h-3 text-muted-foreground" />
                        </div>
                        <div>
                          <p className="text-sm font-medium">{log.action}</p>
                          <p className="text-xs text-muted-foreground">{format(new Date(log.created_at), 'MM/dd HH:mm')}</p>
                        </div>
                      </div>
                    ))}
                    {auditLogs.length === 0 && <p className="text-center py-4 text-sm text-muted-foreground italic">監査ログはまだありません。</p>}
                  </div>
                </div>
              </div>
            </div>
          )}

          {view === 'employees' && (
            <div className="space-y-6 animate-in fade-in slide-in-from-right-2 duration-500">
              <div className="flex items-center justify-between">
                <div>
                  <h2 className="text-2xl font-bold">従業員管理</h2>
                  <p className="text-muted-foreground mt-1 text-sm">従業員情報とカード紐付けを管理します。</p>
                </div>
              </div>

              <div className="grid grid-cols-1 gap-6 xl:grid-cols-2">
                <form className="card p-5 space-y-4" onSubmit={handleEmployeeSubmit}>
                  <div className="flex items-center gap-2">
                    <Plus className="w-5 h-5 text-brand" />
                    <h3 className="font-semibold">従業員を追加</h3>
                  </div>
                  <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
                    <label className="space-y-2 text-sm">
                      <span className="font-medium">氏名</span>
                      <input
                        className="w-full rounded-lg border border-border bg-background px-3 py-2"
                        value={employeeForm.display_name}
                        onChange={(event) => setEmployeeForm((current) => ({ ...current, display_name: event.target.value }))}
                        required
                      />
                    </label>
                    <label className="space-y-2 text-sm">
                      <span className="font-medium">雇用区分</span>
                      <input
                        className="w-full rounded-lg border border-border bg-background px-3 py-2"
                        value={employeeForm.employment_type}
                        onChange={(event) => setEmployeeForm((current) => ({ ...current, employment_type: event.target.value }))}
                        required
                      />
                    </label>
                    <label className="space-y-2 text-sm">
                      <span className="font-medium">所属</span>
                      <input
                        className="w-full rounded-lg border border-border bg-background px-3 py-2"
                        value={employeeForm.affiliation}
                        onChange={(event) => setEmployeeForm((current) => ({ ...current, affiliation: event.target.value }))}
                      />
                    </label>
                    <label className="space-y-2 text-sm">
                      <span className="font-medium">備考</span>
                      <input
                        className="w-full rounded-lg border border-border bg-background px-3 py-2"
                        value={employeeForm.note}
                        onChange={(event) => setEmployeeForm((current) => ({ ...current, note: event.target.value }))}
                      />
                    </label>
                  </div>
                  <button className="rounded-lg bg-brand px-4 py-2 text-sm font-medium text-brand-foreground" type="submit">
                    従業員を追加
                  </button>
                </form>

                <form className="card p-5 space-y-4" onSubmit={handleCardBindSubmit}>
                  <div className="flex items-center gap-2">
                    <CreditCard className="w-5 h-5 text-brand" />
                    <h3 className="font-semibold">カードを紐付け</h3>
                  </div>
                  <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
                    <label className="space-y-2 text-sm">
                      <span className="font-medium">カードID</span>
                      <input
                        className="w-full rounded-lg border border-border bg-background px-3 py-2"
                        value={cardBindForm.card_identifier}
                        onChange={(event) => setCardBindForm((current) => ({ ...current, card_identifier: event.target.value }))}
                        required
                      />
                    </label>
                    <label className="space-y-2 text-sm">
                      <span className="font-medium">従業員</span>
                      <select
                        className="w-full rounded-lg border border-border bg-background px-3 py-2"
                        value={cardBindForm.employee_id}
                        onChange={(event) => setCardBindForm((current) => ({ ...current, employee_id: event.target.value }))}
                        required
                      >
                        {employees.map((employee) => (
                          <option key={employee.id} value={employee.id}>
                            {employee.display_name}
                          </option>
                        ))}
                      </select>
                    </label>
                  </div>
                  <button
                    className="rounded-lg bg-brand px-4 py-2 text-sm font-medium text-brand-foreground disabled:opacity-50"
                    disabled={employees.length === 0}
                    type="submit"
                  >
                    紐付ける
                  </button>
                </form>
              </div>

              <div className="card overflow-x-auto">
                <table className="w-full text-left">
                  <thead className="bg-muted/50 border-b border-border">
                    <tr>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">氏名</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">所属</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">雇用区分</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">状態</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground text-right">操作</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border">
                    {employees.map(emp => (
                      <tr key={emp.id} className="hover:bg-accent/30 transition-colors group">
                        <td className="px-6 py-4">
                          <div className="flex items-center gap-3">
                            <div className="w-8 h-8 rounded-full bg-brand/10 text-brand flex items-center justify-center font-bold text-xs">
                              {emp.display_name.charAt(0)}
                            </div>
                            <span className="font-medium">{emp.display_name}</span>
                          </div>
                        </td>
                        <td className="px-6 py-4 text-sm text-muted-foreground">{emp.affiliation || '-'}</td>
                        <td className="px-6 py-4 text-sm font-mono text-muted-foreground">{emp.employment_type}</td>
                        <td className="px-6 py-4">
                          <span className={cn(
                            "inline-flex items-center px-2.5 py-0.5 rounded-full text-xs font-medium",
                            emp.is_active ? "bg-green-100 text-green-800 dark:bg-green-900/30 dark:text-green-400" : "bg-red-100 text-red-800 dark:bg-red-900/30 dark:text-red-400"
                          )}>
                            {emp.is_active ? '有効' : '無効'}
                          </span>
                        </td>
                        <td className="px-6 py-4 text-right">
                          <button
                            className="inline-flex items-center gap-2 rounded-lg border border-border px-3 py-2 text-sm text-muted-foreground hover:text-foreground disabled:opacity-50"
                            disabled={!emp.is_active}
                            onClick={() => handleDeactivateEmployee(emp.id)}
                            type="button"
                          >
                            <UserMinus className="w-4 h-4" />
                            無効化
                          </button>
                        </td>
                      </tr>
                    ))}
                    {employees.length === 0 && (
                      <tr>
                        <td colSpan={5} className="px-6 py-8 text-center text-sm text-muted-foreground">
                          従業員はまだ登録されていません。
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
          )}

          {view === 'attendance' && (
            <div className="space-y-6 animate-in fade-in slide-in-from-right-2 duration-500">
              <div className="flex flex-col gap-4 lg:flex-row lg:items-end lg:justify-between">
                <div>
                  <h2 className="text-2xl font-bold">勤怠</h2>
                  <p className="text-muted-foreground mt-1 text-sm">従業員ごとの月次勤怠を締め期間で確認します。</p>
                </div>
                <div className="flex flex-col gap-3 sm:flex-row">
                  <label className="flex flex-col gap-2 text-sm">
                    <span className="font-medium text-muted-foreground">従業員</span>
                    <select
                      aria-label="従業員"
                      className="rounded-lg border border-border bg-background px-3 py-2"
                      value={selectedEmployeeId}
                      onChange={(event) => setSelectedEmployeeId(event.target.value)}
                    >
                      {employees.map((employee) => (
                        <option key={employee.id} value={employee.id}>
                          {employee.display_name}
                        </option>
                      ))}
                    </select>
                  </label>
                  <label className="flex flex-col gap-2 text-sm">
                    <span className="font-medium text-muted-foreground">対象月</span>
                    <input
                      aria-label="対象月"
                      type="month"
                      className="rounded-lg border border-border bg-background px-3 py-2"
                      value={selectedYearMonth}
                      onChange={(event) => setSelectedYearMonth(event.target.value)}
                    />
                  </label>
                </div>
              </div>

              {monthlyAttendance && (
                <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
                  <div className="card p-5">
                    <p className="text-sm text-muted-foreground">対象期間</p>
                    <p className="mt-2 text-lg font-semibold">
                      {monthlyAttendance.period_start} から {monthlyAttendance.period_end}
                    </p>
                  </div>
                  <div className="card p-5">
                    <p className="text-sm text-muted-foreground">総労働時間</p>
                    <p className="mt-2 text-lg font-semibold">{formatMinutes(monthlyAttendance.total_work_minutes)}</p>
                  </div>
                  <div className="card p-5">
                    <p className="text-sm text-muted-foreground">勤怠日数</p>
                    <p className="mt-2 text-lg font-semibold">{monthlyAttendance.days.length}日</p>
                  </div>
                </div>
              )}

              <div className="card">
                <table className="w-full text-left">
                  <thead className="bg-muted/50 border-b border-border">
                    <tr>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">日付</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">打刻</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">勤務時間</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">状態</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border">
                    {isAttendanceLoading && (
                      <tr>
                        <td colSpan={4} className="px-6 py-8 text-center text-sm text-muted-foreground">
                          月次勤怠を読み込んでいます...
                        </td>
                      </tr>
                    )}
                    {!isAttendanceLoading && monthlyAttendance?.days.map((day) => (
                      <tr key={day.date} className="hover:bg-accent/30 transition-colors">
                        <td className="px-6 py-4 text-sm font-mono text-muted-foreground">
                          {day.date}
                        </td>
                        <td className="px-6 py-4 text-sm">
                          <div className="space-y-1">
                            {day.events.map((event) => (
                              <div key={event.id} className="flex items-center gap-2 text-muted-foreground">
                                <span className="font-mono">{format(new Date(event.occurred_at), 'HH:mm')}</span>
                                <span>{formatPunchEventLabel(event.event_type)}</span>
                              </div>
                            ))}
                          </div>
                        </td>
                        <td className="px-6 py-4 text-sm font-semibold">
                          {formatMinutes(day.work_minutes)}
                        </td>
                        <td className="px-6 py-4 text-sm">
                          <div className="flex items-center gap-2">
                            <span>{formatAttendanceStatus(day.status)}</span>
                            {day.has_inconsistency && (
                              <span className="rounded bg-amber-100 px-2 py-1 text-xs font-medium text-amber-700">
                                要確認
                              </span>
                            )}
                          </div>
                        </td>
                      </tr>
                    ))}
                    {!isAttendanceLoading && monthlyAttendance && monthlyAttendance.days.length === 0 && (
                      <tr>
                        <td colSpan={4} className="px-6 py-8 text-center text-sm text-muted-foreground">
                          この期間の勤怠はありません。
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
          )}

          {view === 'requests' && (
            <div className="space-y-6 animate-in fade-in slide-in-from-right-2 duration-500">
              <div>
                <h2 className="text-2xl font-bold">修正申請</h2>
                <p className="text-muted-foreground mt-1 text-sm">
                  打刻修正は申請の承認・却下として処理します。
                </p>
              </div>

              <div className="card overflow-x-auto">
                <table className="w-full text-left">
                  <thead className="bg-muted/50 border-b border-border">
                    <tr>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">申請日時</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">従業員</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">対象日</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">内容</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">レビュー</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border">
                    {isRequestLoading && (
                      <tr>
                        <td colSpan={5} className="px-6 py-8 text-center text-sm text-muted-foreground">
                          修正申請を読み込んでいます...
                        </td>
                      </tr>
                    )}
                    {!isRequestLoading && attendanceRequests.map((request) => {
                      const details = formatAttendanceRequest(request);
                      return (
                        <tr key={request.id} className="hover:bg-accent/30 transition-colors">
                          <td className="px-6 py-4 text-sm text-muted-foreground">
                            {format(new Date(request.requested_at || request.created_at), 'yyyy-MM-dd HH:mm')}
                          </td>
                          <td className="px-6 py-4 text-sm font-medium">
                            {employees.find((employee) => employee.id === request.employee_id)?.display_name || request.employee_id}
                          </td>
                          <td className="px-6 py-4 text-sm text-muted-foreground">{details.targetDate}</td>
                          <td className="px-6 py-4 text-sm">
                            <p className="font-medium">{details.title}</p>
                            <p className="mt-1 text-muted-foreground">{details.detail}</p>
                          </td>
                          <td className="px-6 py-4">
                            <div className="flex min-w-72 flex-col gap-2">
                              <input
                                aria-label="レビューコメント"
                                className="rounded-lg border border-border bg-background px-3 py-2 text-sm"
                                placeholder="レビューコメント（任意）"
                                value={reviewNotes[request.id] || ''}
                                onChange={(event) => setReviewNotes((current) => ({ ...current, [request.id]: event.target.value }))}
                              />
                              <div className="flex gap-2">
                                <button
                                  type="button"
                                  className="rounded-lg bg-brand px-3 py-2 text-sm font-medium text-brand-foreground"
                                  onClick={() => handleReviewAttendanceRequest(request.id, 'approve')}
                                >
                                  承認
                                </button>
                                <button
                                  type="button"
                                  className="rounded-lg border border-border px-3 py-2 text-sm font-medium hover:bg-accent"
                                  onClick={() => handleReviewAttendanceRequest(request.id, 'reject')}
                                >
                                  却下
                                </button>
                              </div>
                            </div>
                          </td>
                        </tr>
                      );
                    })}
                    {!isRequestLoading && attendanceRequests.length === 0 && (
                      <tr>
                        <td colSpan={5} className="px-6 py-8 text-center text-sm text-muted-foreground">
                          承認待ちの修正申請はありません。
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
          )}

          {view === 'audit' && (
             <div className="space-y-6 animate-in fade-in slide-in-from-right-2 duration-500">
               <div>
                  <h2 className="text-2xl font-bold">監査ログ</h2>
                  <p className="text-muted-foreground mt-1 text-sm">管理操作の履歴を確認します。</p>
                </div>
              <div className="card">
                <table className="w-full text-left font-mono text-xs">
                  <thead className="bg-muted/50 border-b border-border">
                    <tr>
                      <th className="px-6 py-4 font-semibold text-muted-foreground">日時</th>
                      <th className="px-6 py-4 font-semibold text-muted-foreground">実行者</th>
                      <th className="px-6 py-4 font-semibold text-muted-foreground">操作</th>
                      <th className="px-6 py-4 font-semibold text-muted-foreground">対象</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border">
                    {auditLogs.map(log => (
                      <tr key={log.id} className="hover:bg-accent/30">
                        <td className="px-6 py-4 text-muted-foreground">{format(new Date(log.created_at), 'yyyy-MM-dd HH:mm:ss')}</td>
                        <td className="px-6 py-4 font-bold">{log.actor_type}</td>
                        <td className="px-6 py-4 text-brand">{log.action}</td>
                        <td className="px-6 py-4">
                          {log.target_id ? `${log.target_type}:${log.target_id}` : log.target_type}
                        </td>
                      </tr>
                    ))}
                    {auditLogs.length === 0 && (
                      <tr>
                        <td colSpan={4} className="px-6 py-8 text-center text-sm text-muted-foreground">
                          監査ログはまだありません。
                        </td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </div>
      </main>
    </div>
  );
}
