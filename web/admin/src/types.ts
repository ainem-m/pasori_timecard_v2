export interface Employee {
  id: string;
  display_name: string;
  employment_type: string;
  affiliation?: string;
  note?: string;
  is_active: boolean;
  created_at: string;
}

export interface PunchEvent {
  id: string;
  employee_id: string;
  event_type:
    | 'clock_in'
    | 'clock_out'
    | 'break_start'
    | 'break_end'
    | 'temporary_out'
    | 'temporary_return'
    | 'manual_correction';
  occurred_at: string;
  source: string;
}

export interface AuditLog {
  id: string;
  actor_type: string;
  action: string;
  target_type: string;
  target_id?: string;
  created_at: string;
}

export interface LoginFormState {
  username: string;
  password: string;
}

export interface AttendanceDay {
  date: string;
  events: PunchEvent[];
  work_minutes: number;
  has_inconsistency: boolean;
  status: 'unconfirmed' | 'confirmed' | 'locked';
}

export interface MonthlyAttendance {
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

export interface EmployeeFormState {
  display_name: string;
  employment_type: string;
  affiliation: string;
  note: string;
}

export interface CardBindFormState {
  card_identifier: string;
  employee_id: string;
}

export interface AttendanceRequest {
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
