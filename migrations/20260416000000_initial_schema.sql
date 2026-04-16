-- Initial schema for pasori_timecard_v2

-- 従業員
CREATE TABLE employee (
    id TEXT PRIMARY KEY, -- UUID v7
    display_name TEXT NOT NULL,
    employment_type TEXT NOT NULL, -- 'regular', 'part_time', etc.
    affiliation TEXT,
    is_active BOOLEAN NOT NULL DEFAULT 1,
    note TEXT,
    created_at TEXT NOT NULL, -- Asia/Tokyo Zoned ISO8601
    updated_at TEXT NOT NULL
);

-- IC カード
CREATE TABLE card (
    id TEXT PRIMARY KEY,
    employee_id TEXT NOT NULL REFERENCES employee(id),
    card_identifier TEXT NOT NULL UNIQUE, -- FeliCa IDm hex
    card_label TEXT,
    is_active BOOLEAN NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 打刻イベント
CREATE TABLE punch_event (
    id TEXT PRIMARY KEY, -- Terminal 側で生成した UUID v7
    employee_id TEXT NOT NULL REFERENCES employee(id),
    card_id TEXT REFERENCES card(id),
    event_type TEXT NOT NULL, -- 'clock_in', 'clock_out', etc.
    occurred_at TEXT NOT NULL, -- 打刻時刻
    server_recorded_at TEXT NOT NULL, -- サーバー受信時刻
    source TEXT NOT NULL, -- 'nfc', 'manual', 'import', 'local_cached'
    correction_reason TEXT,
    deleted_at TEXT, -- ソフトデリート用
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- シフト種別
CREATE TABLE shift_type (
    id TEXT PRIMARY KEY,
    code TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    planned_start_time TEXT, -- 'HH:MM'
    planned_end_time TEXT,
    default_break_minutes INTEGER,
    color TEXT NOT NULL DEFAULT '#ffffff',
    is_active BOOLEAN NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- シフト割り当て
CREATE TABLE shift_assignment (
    id TEXT PRIMARY KEY,
    employee_id TEXT NOT NULL REFERENCES employee(id),
    date TEXT NOT NULL, -- 'YYYY-MM-DD'
    shift_type_id TEXT NOT NULL REFERENCES shift_type(id),
    planned_start_at TEXT,
    planned_end_at TEXT,
    note TEXT,
    status TEXT NOT NULL, -- 'draft', 'published', 'finalized'
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(employee_id, date)
);

-- 管理者
CREATE TABLE admin_user (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    display_name TEXT NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 管理セッション
CREATE TABLE admin_session (
    id TEXT PRIMARY KEY,
    admin_user_id TEXT NOT NULL REFERENCES admin_user(id),
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- 外部アカウント連携 (LINE WORKS等)
CREATE TABLE external_account (
    id TEXT PRIMARY KEY,
    employee_id TEXT NOT NULL REFERENCES employee(id),
    provider TEXT NOT NULL, -- 'lineworks'
    external_user_id TEXT NOT NULL,
    external_domain_id TEXT,
    is_verified BOOLEAN NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(provider, external_user_id)
);

-- 修正申請・照会履歴
CREATE TABLE attendance_request (
    id TEXT PRIMARY KEY,
    employee_id TEXT NOT NULL REFERENCES employee(id),
    request_type TEXT NOT NULL, -- 'correction', 'missing_in', etc.
    requested_payload_json TEXT NOT NULL,
    status TEXT NOT NULL, -- 'requested', 'approved', 'rejected', etc.
    requested_via TEXT NOT NULL, -- 'lineworks', 'ui'
    requested_at TEXT NOT NULL,
    reviewed_by_admin_user_id TEXT REFERENCES admin_user(id),
    reviewed_at TEXT,
    review_note TEXT,
    applied_event_id TEXT REFERENCES punch_event(id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 監査ログ
CREATE TABLE audit_log (
    id TEXT PRIMARY KEY,
    actor_type TEXT NOT NULL, -- 'admin', 'employee', 'system', 'terminal'
    actor_id TEXT,
    action TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT,
    before_json TEXT,
    after_json TEXT,
    metadata_json TEXT,
    created_at TEXT NOT NULL
);

-- 登録済み打刻端末
CREATE TABLE terminal (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    api_token_hash TEXT NOT NULL UNIQUE,
    last_seen_at TEXT,
    is_active BOOLEAN NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- インデックス
CREATE INDEX idx_punch_employee_date ON punch_event(employee_id, occurred_at);
CREATE INDEX idx_shift_employee_date ON shift_assignment(employee_id, date);
CREATE INDEX idx_audit_created_at ON audit_log(created_at);
