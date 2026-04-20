import { useEffect, useState, useCallback, useRef } from 'react'
import { listen } from '@tauri-apps/api/event'
import { invoke } from '@tauri-apps/api/core'

// --- Types ---
type ReaderStatus = 'Disconnected' | 'Connecting' | 'Ready' | { Error: string };
type PunchType = 'ClockIn' | 'ClockOut';

interface Employee {
  id: string;
  display_name: string;
}

interface PunchEvent {
  event_type: PunchType;
  occurred_at: string;
}

interface RegisteredResponse {
  employee: Employee;
  recent_events: PunchEvent[];
  suggested_type: PunchType;
}

type ResolveCardResponse =
  | ({ status: 'registered' } & RegisteredResponse)
  | { status: 'unregistered'; card_id: string };

type ScanResult = 
  | { status: 'registered', data: RegisteredResponse }
  | { status: 'unregistered', card_id: string }
  | { status: 'error', message: string }
  | { status: 'success', employee_name: string, punch_type: PunchType };

interface ClockStatus {
  is_synced: boolean;
  offset_seconds: number;
}

// --- Icons (Inline SVG) ---
const Icons = {
  Clock: () => (
    <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
  ),
  User: () => (
    <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M19 21v-2a4 4 0 0 0-4-4H9a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/></svg>
  ),
  Alert: () => (
    <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
  ),
  Check: () => (
    <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M20 6L9 17l-5-5"/></svg>
  ),
  Card: () => (
    <svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round"><rect width="20" height="14" x="2" y="5" rx="2"/><line x1="2" x2="22" y1="10" y2="10"/></svg>
  )
};

const COUNTDOWN_MAX = 30;

function parseReaderStatus(status: string): ReaderStatus {
  if (status === 'Disconnected' || status === 'Connecting' || status === 'Ready') {
    return status;
  }

  return { Error: status };
}

function App() {
  const [status, setStatus] = useState<ReaderStatus>('Connecting');
  const [scanResult, setScanResult] = useState<ScanResult | null>(null);
  const [countdown, setCountdown] = useState<number | null>(null);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [clockError, setClockError] = useState<string | null>(null);
  const [currentTime, setCurrentTime] = useState(new Date());
  const [suggestedType, setSuggestedType] = useState<PunchType>('ClockIn');
  
  // Long press handling
  const [isPressing, setIsPressing] = useState(false);
  const longPressTimer = useRef<number | null>(null);

  // Clock Update
  useEffect(() => {
    const timer = setInterval(() => setCurrentTime(new Date()), 1000);
    return () => clearInterval(timer);
  }, []);

  // Sync Check
  const checkSync = useCallback(async () => {
    try {
      const res = await invoke<ClockStatus>('check_clock_sync');
      if (!res.is_synced) {
        setClockError(`時刻が同期されていません (差分: ${res.offset_seconds}秒)。管理者に連絡してください。`);
      } else {
        setClockError(null);
      }
    } catch (e) {
      // Offline fallback: we might not be able to check sync, but let's not block completely if we can't reach server
      console.warn('Clock sync check failed:', e);
    }
  }, []);

  useEffect(() => {
    checkSync();
    const interval = setInterval(checkSync, 60000);
    return () => clearInterval(interval);
  }, [checkSync]);

  // Reader Status
  useEffect(() => {
    const checkStatus = async () => {
      try {
        const s = await invoke<string>('get_reader_status');
        setStatus(parseReaderStatus(s));
      } catch (e) {
        setStatus({ Error: String(e) });
      }
    };
    checkStatus();
    const interval = setInterval(checkStatus, 5000);
    return () => clearInterval(interval);
  }, []);

  // Punch Submission
  const submitPunch = useCallback(async (data: RegisteredResponse, overrideType?: PunchType) => {
    if (isSubmitting || clockError) return;
    setIsSubmitting(true);
    
    try {
      const punchId = crypto.randomUUID(); 
      const now = new Date().toISOString();
      const type = overrideType || suggestedType;
      
      await invoke('submit_punch', {
        req: {
          punch_id: punchId,
          card_id: scannedCardIdRef.current,
          event_type: type,
          occurred_at: now,
          source: 'terminal',
        }
      });
      
      setScanResult({ status: 'success', employee_name: data.employee.display_name, punch_type: type });
      setCountdown(null);
      
      // Auto return to idle after 3s
      setTimeout(() => setScanResult(null), 3000);
    } catch (e) {
      console.error(e);
      setScanResult({ status: 'error', message: '打刻の送信に失敗しました。' });
      setTimeout(() => setScanResult(null), 5000);
    } finally {
      setIsSubmitting(false);
    }
  }, [isSubmitting, clockError, suggestedType]);

  // Card Scan Listener
  const scannedCardIdRef = useRef<string>('');
  
  useEffect(() => {
    const unlisten = listen<string>('card-scanned', async (event) => {
      if (clockError) return;
      
      scannedCardIdRef.current = event.payload;
      setScanResult(null);
      setCountdown(null);
      
      try {
        const res = await invoke<ResolveCardResponse>('resolve_card', { cardId: event.payload });
        if (res.status === 'registered') {
          const data: RegisteredResponse = {
            employee: res.employee,
            recent_events: res.recent_events,
            suggested_type: res.suggested_type,
          };
          setScanResult({ status: 'registered', data });
          setSuggestedType(data.suggested_type);
          setCountdown(COUNTDOWN_MAX);
        } else {
          setScanResult({ status: 'unregistered', card_id: res.card_id });
          setTimeout(() => setScanResult(null), 5000);
        }
      } catch {
        setScanResult({ status: 'error', message: 'サーバーまたはカードの解析に失敗しました。' });
      }
    });

    return () => { unlisten.then(f => f()); };
  }, [clockError]);

  // Countdown
  useEffect(() => {
    if (countdown === null || countdown <= 0) {
      if (countdown === 0 && scanResult?.status === 'registered') {
        submitPunch(scanResult.data);
      }
      return;
    }
    const timer = setTimeout(() => setCountdown(countdown - 1), 1000);
    return () => clearTimeout(timer);
  }, [countdown, scanResult, submitPunch]);

  // Interaction handlers
  const handleToggleType = () => {
    setSuggestedType(prev => prev === 'ClockIn' ? 'ClockOut' : 'ClockIn');
    setCountdown(COUNTDOWN_MAX); // Reset countdown on change
  };

  const startPress = () => {
    if (scanResult?.status !== 'registered' || isSubmitting) return;
    setIsPressing(true);
    longPressTimer.current = window.setTimeout(() => {
      setIsPressing(false);
      if (scanResult?.status === 'registered') {
        submitPunch(scanResult.data);
      }
    }, 1000);
  };

  const endPress = () => {
    setIsPressing(false);
    if (longPressTimer.current) {
      clearTimeout(longPressTimer.current);
      longPressTimer.current = null;
    }
  };

  // --- Render Helpers ---

  if (clockError) {
    return (
      <div className="flex flex-col items-center justify-center min-h-screen p-8 text-white space-y-8 animate-in">
        <div className="w-24 h-24 text-red-500 fill-red-500/10"><Icons.Alert /></div>
        <h1 className="text-5xl font-black text-red-500">時刻同期エラー</h1>
        <p className="text-2xl text-center max-w-xl opacity-80 leading-relaxed">{clockError}</p>
      </div>
    );
  }

  return (
    <div className="relative flex flex-col items-center justify-center min-h-screen p-8 text-white overflow-hidden">
      {/* Background Decor */}
      <div className="absolute top-[-10%] left-[-10%] w-[40%] h-[40%] bg-primary-blue/10 blur-[120px] rounded-full" />
      <div className="absolute bottom-[-10%] right-[-10%] w-[40%] h-[40%] bg-primary-orange/10 blur-[120px] rounded-full" />

      {/* Header Info */}
      <header className="absolute top-12 left-12 flex items-center gap-6 opacity-40">
        <div className="flex flex-col">
          <span className="text-4xl font-black tracking-tighter">ATTENDANCE KIOSK</span>
          <div className="flex items-center gap-2">
            <div className={`w-2 h-2 rounded-full ${status === 'Ready' ? 'bg-green-400' : 'bg-red-400'}`} />
            <span className="text-xs font-bold uppercase tracking-widest">
              {typeof status === 'string' ? status : 'Reader Error'}
            </span>
          </div>
        </div>
      </header>

      <div className="absolute top-12 right-12 text-right">
        <div className="text-6xl font-black tabular-nums">{currentTime.toLocaleTimeString('ja-JP', { hour: '2-digit', minute: '2-digit' })}</div>
        <div className="text-xl font-bold opacity-40">{currentTime.toLocaleDateString('ja-JP', { year: 'numeric', month: 'long', day: 'numeric', weekday: 'short' })}</div>
      </div>

      {/* Idle Screen */}
      {!scanResult && (
        <div className="flex flex-col items-center space-y-12 animate-in text-center">
          <div className="glass w-80 h-80 rounded-[4rem] flex items-center justify-center text-primary-blue animate-pulse-slow">
            <Icons.Card />
          </div>
          <div className="space-y-4">
            <h2 className="text-6xl font-black tracking-tight">カードをタッチ</h2>
            <p className="text-2xl font-medium opacity-30">PaSoRi 打刻ターミナル</p>
          </div>
          
          <style>{`
            @keyframes pulse-slow {
              0%, 100% { transform: scale(1); opacity: 1; }
              50% { transform: scale(1.05); opacity: 0.8; }
            }
            .animate-pulse-slow { animation: pulse-slow 3s ease-in-out infinite; }
          `}</style>
        </div>
      )}

      {/* Confirmation Screen */}
      {scanResult?.status === 'registered' && (
        <div className="glass w-full max-w-4xl rounded-[4rem] p-16 space-y-12 animate-in relative overflow-hidden">
          {/* Progress bar background for countdown */}
          <div 
            className="absolute bottom-0 left-0 h-2 bg-white/10 transition-all duration-1000"
            style={{ width: `${(countdown || 0) / COUNTDOWN_MAX * 100}%` }}
          />

          <div className="flex items-center gap-8 border-b border-white/5 pb-12">
            <div className="w-24 h-24 bg-white/5 rounded-full flex items-center justify-center text-white/40">
              <Icons.User />
            </div>
            <div className="space-y-1">
              <p className="text-2xl font-bold opacity-30">WELCOME back,</p>
              <h2 className="text-7xl font-black">{scanResult.data.employee.display_name} <span className="text-3xl font-medium opacity-30">ID:{scanResult.data.employee.id.slice(0,8)}</span></h2>
            </div>
          </div>

          <div className="grid grid-cols-2 gap-12 items-center">
            <div className="space-y-8">
               <div className="flex flex-col gap-4">
                  <span className="text-xs font-bold uppercase tracking-[0.2em] opacity-30">Suggested Action</span>
                  <div 
                    onClick={handleToggleType}
                    className={`cursor-pointer group relative py-10 px-12 rounded-[2.5rem] flex items-center justify-center text-8xl font-black transition-all duration-500 overflow-hidden ${
                      suggestedType === 'ClockIn' ? 'bg-primary-blue text-white shadow-[0_20px_60px_-15px_rgba(59,130,246,0.5)]' : 'bg-primary-orange text-white shadow-[0_20px_60px_-15px_rgba(249,115,22,0.5)]'
                    }`}
                  >
                    {suggestedType === 'ClockIn' ? '出勤' : '退勤'}
                    <div className="absolute inset-0 bg-white/20 opacity-0 group-hover:opacity-100 transition-opacity flex items-center justify-center text-xl font-bold">
                       TAP TO CHANGE
                    </div>
                  </div>
               </div>
            </div>

            <div className="flex flex-col items-center justify-center space-y-6">
              <div className="relative w-48 h-48 flex items-center justify-center">
                <svg className="absolute inset-0 w-full h-full -rotate-90">
                  <circle cx="96" cy="96" r="88" stroke="white" strokeWidth="4" fill="transparent" opacity="0.1" />
                  <circle 
                    cx="96" cy="96" r="88" 
                    stroke="white" strokeWidth="8" fill="transparent" 
                    strokeDasharray={552} 
                    strokeDashoffset={552 * (1 - (countdown || 0) / COUNTDOWN_MAX)}
                    strokeLinecap="round"
                    className="countdown-ring"
                  />
                </svg>
                <span className="text-7xl font-black tabular-nums">{countdown}</span>
              </div>
              <p className="text-xl font-bold opacity-30 uppercase tracking-widest">Auto confirming</p>
            </div>
          </div>

          {/* Controls */}
          <div className="flex gap-6 pt-12">
            <button 
              onClick={() => setScanResult(null)}
              className="flex-1 py-10 rounded-[2.5rem] bg-white/5 hover:bg-white/10 text-3xl font-black transition-all"
            >
              CANCEL
            </button>
            <button 
              onMouseDown={startPress}
              onMouseUp={endPress}
              onMouseLeave={endPress}
              onTouchStart={startPress}
              onTouchEnd={endPress}
              className={`flex-1 py-10 rounded-[2.5rem] text-4xl font-black relative overflow-hidden transition-all duration-300 ${isSubmitting ? 'scale-95 opacity-50' : 'hover:scale-[1.02] shadow-2xl'} ${
                suggestedType === 'ClockIn' ? 'bg-white text-primary-blue' : 'bg-white text-primary-orange'
              }`}
            >
               <div 
                 className="absolute bottom-0 left-0 h-full bg-black/10 transition-all duration-100"
                 style={{ width: isPressing ? '100%' : '0%', transitionDuration: isPressing ? '1000ms' : '0ms' }}
               />
               {isSubmitting ? 'SENDING...' : 'CONFIRM'}
            </button>
          </div>

          {/* History */}
          <div className="pt-8 opacity-20 hover:opacity-100 transition-opacity">
            <p className="text-xs font-bold uppercase tracking-widest mb-6">Recent Records</p>
            <div className="grid grid-cols-5 gap-4">
              {scanResult.data.recent_events.slice(0, 5).map((ev, i) => (
                <div key={i} className="flex flex-col gap-1">
                  <span className="text-lg font-black">{ev.event_type === 'ClockIn' ? '出勤' : '退勤'}</span>
                  <span className="text-xs font-bold opacity-60 font-mono">{new Date(ev.occurred_at).toLocaleTimeString('ja-JP', { hour: '2-digit', minute: '2-digit' })}</span>
                </div>
              ))}
            </div>
          </div>
        </div>
      )}

      {/* Success Screen */}
      {scanResult?.status === 'success' && (
        <div className="flex flex-col items-center space-y-12 animate-in">
          <div className="w-32 h-32 bg-green-500 rounded-full flex items-center justify-center text-white scale-125 shadow-[0_0_80px_rgba(34,197,94,0.4)]">
            <Icons.Check />
          </div>
          <div className="text-center space-y-4">
            <h2 className="text-8xl font-black tracking-tight">DONE!</h2>
            <p className="text-3xl font-bold opacity-60">
              {scanResult.employee_name} さん、{scanResult.punch_type === 'ClockIn' ? 'おはようございます' : 'お疲れ様でした'}
            </p>
          </div>
        </div>
      )}

      {/* Unregistered / Error */}
      {(scanResult?.status === 'unregistered' || scanResult?.status === 'error') && (
        <div className="glass max-w-2xl rounded-[3rem] p-16 flex flex-col items-center space-y-10 animate-in text-center">
          <div className="w-24 h-24 text-red-500 fill-red-500/10"><Icons.Alert /></div>
          <div className="space-y-4 text-center">
            <h2 className="text-5xl font-black text-red-500">
              {scanResult.status === 'unregistered' ? '未登録カード' : 'システムエラー'}
            </h2>
            <p className="text-2xl opacity-60 leading-relaxed font-bold">
              {scanResult.status === 'unregistered' 
                ? 'このカードは登録されていません。' 
                : scanResult.message}
            </p>
            {scanResult.status === 'unregistered' && (
              <code className="block bg-white/5 py-4 px-6 rounded-2xl text-xl font-mono text-white/40">ID: {scanResult.card_id}</code>
            )}
          </div>
          <button 
            onClick={() => setScanResult(null)}
            className="w-full py-8 bg-white/5 hover:bg-white/10 rounded-3xl font-black text-2xl"
          >
            戻る
          </button>
        </div>
      )}
    </div>
  )
}

export default App
