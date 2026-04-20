import { type FormEvent, useEffect, useState } from 'react';
import { 
  Users, 
  Clock, 
  ShieldAlert, 
  LayoutDashboard, 
  Search, 
  Plus, 
  Settings,
  Bell,
  Menu,
  ChevronRight,
  UserPlus,
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
  is_active: boolean;
  created_at: string;
}

interface PunchEvent {
  id: string;
  employee_id: string;
  event_type: 'ClockIn' | 'ClockOut';
  occurred_at: string;
  source: string;
}

interface AuditLog {
  id: string;
  actor_type: string;
  action: string;
  target_type: string;
  created_at: string;
}

interface LoginFormState {
  username: string;
  password: string;
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
  const [view, setView] = useState<'dashboard' | 'employees' | 'attendance' | 'audit'>('dashboard');
  const [employees, setEmployees] = useState<Employee[]>([]);
  const [punches, setPunches] = useState<PunchEvent[]>([]);
  const [auditLogs, setAuditLogs] = useState<AuditLog[]>([]);
  const [isLoading, setLoading] = useState(true);
  const [needsLogin, setNeedsLogin] = useState(false);
  const [loginError, setLoginError] = useState<string | null>(null);
  const [loginForm, setLoginForm] = useState<LoginFormState>({ username: '', password: '' });

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
            <h1 className="text-3xl font-bold tracking-tight">Admin Login</h1>
            <p className="text-sm text-muted-foreground">管理画面を利用するにはログインが必要です。</p>
          </div>

          <form className="space-y-4" onSubmit={handleLoginSubmit}>
            <label className="block space-y-2">
              <span className="text-sm font-medium">Username</span>
              <input
                className="w-full rounded-lg border border-border bg-background px-3 py-2"
                value={loginForm.username}
                onChange={(event) => setLoginForm((current) => ({ ...current, username: event.target.value }))}
                autoComplete="username"
              />
            </label>
            <label className="block space-y-2">
              <span className="text-sm font-medium">Password</span>
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
              {isLoading ? 'Signing in...' : 'Sign in'}
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
          <SidebarItem icon={LayoutDashboard} label="Dashboard" active={view === 'dashboard'} onClick={() => setView('dashboard')} />
          <SidebarItem icon={Users} label="Employees" active={view === 'employees'} onClick={() => setView('employees')} />
          <SidebarItem icon={Clock} label="Attendance" active={view === 'attendance'} onClick={() => setView('attendance')} />
          <SidebarItem icon={ShieldAlert} label="Audit Logs" active={view === 'audit'} onClick={() => setView('audit')} />
        </nav>

        <div className="mt-auto pt-4 border-t border-border">
          <SidebarItem icon={Settings} label="Settings" onClick={() => {}} />
        </div>
      </aside>

      {/* Main Content */}
      <main className="flex-1 flex flex-col min-w-0 overflow-y-auto">
        {/* Header */}
        <header className="h-16 border-b border-border flex items-center justify-between px-8 bg-background/80 backdrop-blur-md sticky top-0 z-10">
          <h1 className="text-lg font-semibold capitalize">
            {view === 'dashboard' ? 'Overview' : view}
          </h1>
          
          <div className="flex items-center gap-4">
            <div className="relative group">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground group-focus-within:text-foreground transition-colors" />
              <input 
                type="text" 
                placeholder="Search..." 
                className="bg-accent h-9 pl-10 pr-4 rounded-full text-sm border-transparent focus:outline-none focus:ring-2 focus:ring-brand/50 w-64 transition-all"
              />
            </div>
            <button className="p-2 rounded-full hover:bg-accent relative transition-colors">
              <Bell className="w-5 h-5" />
              <span className="absolute top-2 right-2 w-2 h-2 bg-brand rounded-full border-2 border-background"></span>
            </button>
            <button
              type="button"
              onClick={handleLogout}
              className="rounded-full border border-border px-4 py-2 text-sm font-medium transition-colors hover:bg-accent"
            >
              Logout
            </button>
            <div className="w-8 h-8 rounded-full bg-accent border border-border"></div>
          </div>
        </header>

        {/* Content Area */}
        <div className="p-8">
          {view === 'dashboard' && (
            <div className="space-y-8 animate-in fade-in slide-in-from-bottom-2 duration-500">
              <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
                <StatCard title="Total Employees" value={employees.length} icon={Users} trend="+2 this month" />
                <StatCard title="Active Today" value={new Set(punches.map(p => p.employee_id)).size} icon={UserPlus} />
                <StatCard title="Total Punches" value={punches.length} icon={Clock} />
                <StatCard title="System Node" value="Healthy" icon={ShieldAlert} />
              </div>

              <div className="grid grid-cols-1 lg:grid-cols-3 gap-8">
                <div className="lg:col-span-2 space-y-4">
                  <div className="flex items-center justify-between">
                    <h2 className="text-xl font-bold">Recent Attendance</h2>
                    <button onClick={() => setView('attendance')} className="text-sm text-brand font-medium hover:underline">View All</button>
                  </div>
                  <div className="card divide-y divide-border">
                    {punches.slice(0, 5).map(punch => (
                      <div key={punch.id} className="p-4 flex items-center justify-between hover:bg-accent/30 transition-colors">
                        <div className="flex items-center gap-3">
                          <div className={cn(
                            "w-2 h-2 rounded-full",
                            punch.event_type === 'ClockIn' ? "bg-green-500" : "bg-red-500"
                          )} />
                          <div>
                            <p className="font-medium text-sm">
                              {employees.find(e => e.id === punch.employee_id)?.display_name || 'Unknown'}
                            </p>
                            <p className="text-xs text-muted-foreground">{punch.event_type}</p>
                          </div>
                      </div>
                      <p className="text-sm text-muted-foreground">{format(new Date(punch.occurred_at), 'HH:mm:ss')}</p>
                    </div>
                    ))}
                    {punches.length === 0 && <p className="p-8 text-center text-muted-foreground italic">No data available</p>}
                  </div>
                </div>

                <div className="space-y-4">
                  <h2 className="text-xl font-bold">Recent Activity</h2>
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
                    {auditLogs.length === 0 && <p className="text-center py-4 text-sm text-muted-foreground italic">Clean logs</p>}
                  </div>
                </div>
              </div>
            </div>
          )}

          {view === 'employees' && (
            <div className="space-y-6 animate-in fade-in slide-in-from-right-2 duration-500">
              <div className="flex items-center justify-between">
                <div>
                  <h2 className="text-2xl font-bold">Employees</h2>
                  <p className="text-muted-foreground mt-1 text-sm">Manage staff and card pairings</p>
                </div>
                <button className="flex items-center gap-2 bg-brand text-brand-foreground px-4 py-2 rounded-lg font-medium shadow-lg shadow-brand/20 hover:scale-105 transition-transform active:scale-95">
                  <Plus className="w-4 h-4" />
                  <span>Add Employee</span>
                </button>
              </div>

              <div className="card overflow-x-auto">
                <table className="w-full text-left">
                  <thead className="bg-muted/50 border-b border-border">
                    <tr>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">Name</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">Affiliation</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">Type</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">Status</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground text-right">Actions</th>
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
                            {emp.is_active ? 'Active' : 'Inactive'}
                          </span>
                        </td>
                        <td className="px-6 py-4 text-right">
                          <button className="text-muted-foreground hover:text-foreground">
                            <Menu className="w-4 h-4" />
                          </button>
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}

          {view === 'attendance' && (
            <div className="space-y-6 animate-in fade-in slide-in-from-right-2 duration-500">
               <div>
                  <h2 className="text-2xl font-bold">Attendance Records</h2>
                  <p className="text-muted-foreground mt-1 text-sm">Full punch history logs</p>
                </div>
              <div className="card">
                <table className="w-full text-left">
                  <thead className="bg-muted/50 border-b border-border">
                    <tr>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">Time</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">Employee</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">Event</th>
                      <th className="px-6 py-4 text-xs font-semibold uppercase tracking-wider text-muted-foreground">Source</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border">
                    {punches.map(p => (
                      <tr key={p.id} className="hover:bg-accent/30 transition-colors">
                        <td className="px-6 py-4 text-sm font-mono text-muted-foreground">
                          {format(new Date(p.occurred_at), 'MM/dd HH:mm:ss')}
                        </td>
                        <td className="px-6 py-4 font-medium">
                          {employees.find(e => e.id === p.employee_id)?.display_name || 'Deleted User'}
                        </td>
                        <td className="px-6 py-4">
                          <span className={cn(
                            "px-2 py-1 rounded text-xs font-bold",
                            p.event_type === 'ClockIn' ? "text-green-500" : "text-red-500"
                          )}>
                            {p.event_type.toUpperCase()}
                          </span>
                        </td>
                        <td className="px-6 py-4 text-xs uppercase text-muted-foreground font-semibold">
                          {p.source}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}

          {view === 'audit' && (
             <div className="space-y-6 animate-in fade-in slide-in-from-right-2 duration-500">
               <div>
                  <h2 className="text-2xl font-bold">Audit Logs</h2>
                  <p className="text-muted-foreground mt-1 text-sm">Administrative action history</p>
                </div>
              <div className="card">
                <table className="w-full text-left font-mono text-xs">
                  <thead className="bg-muted/50 border-b border-border">
                    <tr>
                      <th className="px-6 py-4 font-semibold text-muted-foreground">Timestamp</th>
                      <th className="px-6 py-4 font-semibold text-muted-foreground">Actor</th>
                      <th className="px-6 py-4 font-semibold text-muted-foreground">Action</th>
                      <th className="px-6 py-4 font-semibold text-muted-foreground">Target</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border">
                    {auditLogs.map(log => (
                      <tr key={log.id} className="hover:bg-accent/30">
                        <td className="px-6 py-4 text-muted-foreground">{format(new Date(log.created_at), 'yyyy-MM-dd HH:mm:ss')}</td>
                        <td className="px-6 py-4 font-bold">{log.actor_type}</td>
                        <td className="px-6 py-4 text-brand">{log.action}</td>
                        <td className="px-6 py-4">{log.target_type}:{log.id.slice(0, 8)}</td>
                      </tr>
                    ))}
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
