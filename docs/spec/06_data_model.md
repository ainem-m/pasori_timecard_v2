# 詳細仕様: データモデル

## 原則

- **主キーは UUID v7** (時系列ソート可能、将来の同期に有利)
- `created_at`, `updated_at` は **Asia/Tokyo 保存の TEXT 形式** (例: `2026-04-16T09:15:30+09:00`)
  - SQLite では `TEXT` で保存、Rust 側で `jiff::Zoned` にパース
- 物理削除は行わない。ソフトデリート (`is_active` または `deleted_at`)
- 重要な変更は `audit_log` に append

## マイグレーション

- `sqlx migrate` を使用
- ディレクトリ: `/migrations/`
- 命名: `YYYYMMDDHHMMSS_description.sql`
- 順番厳守、既存マイグレーションの編集禁止 (新規で打ち消しマイグレーションを追加する)

## テーブル

### employee

```sql
CREATE TABLE employee (
    id TEXT PRIMARY KEY NOT NULL,               -- UUID v7
    display_name TEXT NOT NULL,
    employment_type TEXT NOT NULL,              -- 'regular' / 'part_time' / 'doctor' / その他
    affiliation TEXT,
    is_active INTEGER NOT NULL DEFAULT 1,
    note TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_employee_active ON employee(is_active);
```

### card

```sql
CREATE TABLE card (
    id TEXT PRIMARY KEY NOT NULL,
    employee_id TEXT NOT NULL REFERENCES employee(id),
    card_identifier TEXT UNIQUE NOT NULL,       -- FeliCa IDm の hex
    card_label TEXT,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_card_employee ON card(employee_id);
CREATE INDEX idx_card_identifier ON card(card_identifier);
```

### punch_event

```sql
CREATE TABLE punch_event (
    id TEXT PRIMARY KEY NOT NULL,                -- Terminal 生成 UUID v7 (冪等性保証)
    employee_id TEXT NOT NULL REFERENCES employee(id),
    card_id TEXT REFERENCES card(id),            -- card table の UUID、手動修正時は NULL 可
    event_type TEXT NOT NULL,                    -- 'clock_in' / 'clock_out' / 他
    occurred_at TEXT NOT NULL,                   -- 打刻発生時刻 (Asia/Tokyo)
    server_recorded_at TEXT NOT NULL,            -- Server が受信した時刻
    source TEXT NOT NULL,                        -- 'nfc' / 'manual' / 'import' / 'local_cached'
    correction_reason TEXT,
    deleted_at TEXT,                             -- ソフトデリート
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_punch_employee_time ON punch_event(employee_id, occurred_at);
CREATE INDEX idx_punch_occurred ON punch_event(occurred_at);
```

### shift_type

```sql
CREATE TABLE shift_type (
    id TEXT PRIMARY KEY NOT NULL,
    code TEXT UNIQUE NOT NULL,                   -- 'NORMAL' / 'AM' / 'PM' / 'OFF' / 'PAID' / 'SPECIAL' / 'STANDBY'
    display_name TEXT NOT NULL,
    planned_start_time TEXT,                     -- 'HH:MM'
    planned_end_time TEXT,
    default_break_minutes INTEGER,
    color TEXT NOT NULL DEFAULT '#999999',
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

### shift_assignment

```sql
CREATE TABLE shift_assignment (
    id TEXT PRIMARY KEY NOT NULL,
    employee_id TEXT NOT NULL REFERENCES employee(id),
    date TEXT NOT NULL,                          -- 'YYYY-MM-DD'
    shift_type_id TEXT NOT NULL REFERENCES shift_type(id),
    planned_start_at TEXT,
    planned_end_at TEXT,
    note TEXT,
    status TEXT NOT NULL DEFAULT 'draft',        -- 'draft' / 'published' / 'finalized'
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(employee_id, date)
);
CREATE INDEX idx_shift_employee_date ON shift_assignment(employee_id, date);
```

### admin_user

```sql
CREATE TABLE admin_user (
    id TEXT PRIMARY KEY NOT NULL,
    username TEXT UNIQUE NOT NULL,
    password_hash TEXT NOT NULL,                 -- Argon2id
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

### admin_session

```sql
CREATE TABLE admin_session (
    id TEXT PRIMARY KEY NOT NULL,                -- Session token (random 256bit hex)
    admin_user_id TEXT NOT NULL REFERENCES admin_user(id),
    issued_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    last_active_at TEXT NOT NULL,
    user_agent TEXT,
    ip_address TEXT
);
CREATE INDEX idx_session_expires ON admin_session(expires_at);
```

### terminal

```sql
CREATE TABLE terminal (
    id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT NOT NULL,
    api_token_hash TEXT UNIQUE NOT NULL,         -- Argon2id (Bearer token を hash して保存)
    is_active INTEGER NOT NULL DEFAULT 1,
    last_seen_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

### external_account

```sql
CREATE TABLE external_account (
    id TEXT PRIMARY KEY NOT NULL,
    employee_id TEXT NOT NULL REFERENCES employee(id),
    provider TEXT NOT NULL,                      -- 'lineworks'
    external_user_id TEXT NOT NULL,
    external_domain_id TEXT,
    is_verified INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(provider, external_user_id)
);
CREATE INDEX idx_external_employee ON external_account(employee_id);
```

### attendance_request

```sql
CREATE TABLE attendance_request (
    id TEXT PRIMARY KEY NOT NULL,
    employee_id TEXT NOT NULL REFERENCES employee(id),
    request_type TEXT NOT NULL,
    requested_payload_json TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'requested',
    requested_via TEXT NOT NULL,
    requested_at TEXT NOT NULL,
    reviewed_by_admin_user_id TEXT REFERENCES admin_user(id),
    reviewed_at TEXT,
    review_note TEXT,
    applied_event_id TEXT REFERENCES punch_event(id)
);
CREATE INDEX idx_request_status ON attendance_request(status);
CREATE INDEX idx_request_employee ON attendance_request(employee_id);
```

### audit_log

```sql
CREATE TABLE audit_log (
    id TEXT PRIMARY KEY NOT NULL,                -- UUID v7 (時系列ソート)
    actor_type TEXT NOT NULL,                    -- 'admin' / 'employee' / 'system' / 'terminal'
    actor_id TEXT,
    action TEXT NOT NULL,                        -- 'punch.update' など
    target_type TEXT NOT NULL,
    target_id TEXT,
    before_json TEXT,
    after_json TEXT,
    metadata_json TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_audit_action ON audit_log(action);
CREATE INDEX idx_audit_created ON audit_log(created_at);

-- DELETE / UPDATE を物理的に禁止
CREATE TRIGGER audit_log_no_update
BEFORE UPDATE ON audit_log
FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;

CREATE TRIGGER audit_log_no_delete
BEFORE DELETE ON audit_log
FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only');
END;
```

### settings

```sql
CREATE TABLE settings (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL,                         -- JSON
    updated_at TEXT NOT NULL,
    updated_by_admin_user_id TEXT REFERENCES admin_user(id)
);

-- 既定値
INSERT INTO settings(key, value, updated_at) VALUES
    ('cutoff_date', '15', '2026-04-16T00:00:00+09:00'),
    ('ntp_tolerance_seconds', '10', '2026-04-16T00:00:00+09:00'),
    ('auto_commit_grace_seconds', '30', '2026-04-16T00:00:00+09:00'),
    ('long_press_skip_ms', '1000', '2026-04-16T00:00:00+09:00'),
    ('duplicate_scan_ignore_seconds', '5', '2026-04-16T00:00:00+09:00'),
    ('terminal_pull_interval_seconds', '30', '2026-04-16T00:00:00+09:00'),
    ('backup_retention_days', '30', '2026-04-16T00:00:00+09:00'),
    ('admin_session_hours', '24', '2026-04-16T00:00:00+09:00');
```

## Terminal 側 SQLite (local cache)

Terminal は自身の SQLite ファイルを持つ。Server 停止時の fallback 用。

```sql
CREATE TABLE pending_punch (
    punch_id TEXT PRIMARY KEY NOT NULL,          -- UUID v7、Server に送る時もこの ID を使う
    employee_id TEXT NOT NULL,
    card_id TEXT NOT NULL,                       -- card table の UUID
    event_type TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_sync_attempt_at TEXT,
    sync_status TEXT NOT NULL DEFAULT 'pending'  -- 'pending' / 'synced' / 'conflict'
);

CREATE TABLE employee_cache (
    id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT NOT NULL,
    cached_at TEXT NOT NULL
);

CREATE TABLE card_cache (
    card_id TEXT NOT NULL,
    card_identifier TEXT PRIMARY KEY NOT NULL,
    employee_id TEXT NOT NULL,
    cached_at TEXT NOT NULL
);

CREATE TABLE recent_punch_cache (
    punch_id TEXT PRIMARY KEY NOT NULL,
    employee_id TEXT NOT NULL,
    card_id TEXT,
    event_type TEXT NOT NULL,
    occurred_at TEXT NOT NULL,
    cached_at TEXT NOT NULL
);
```

Terminal の SQLite パス:
- macOS: `~/Library/Application Support/pasori-timecard-terminal/local.db`
- Windows: `%APPDATA%\pasori-timecard-terminal\local.db`
- Linux: `~/.local/share/pasori-timecard-terminal/local.db`

## 重要な制約

- `punch_event.id` は **Terminal が生成** する。Server は UNIQUE 制約で冪等性を担保
- `card_id` は常に `card` table の UUID を指す
- FeliCa IDm は `card_identifier` として扱う
- `punch_event.occurred_at` は Terminal 時計由来。`server_recorded_at` は Server 時計由来
- オフライン打刻は `source = 'local_cached'` で識別できるようにする
- オフライン時も online と同じ確認 UI を維持するため、Terminal は recent punches を cache する
- `audit_log` の UPDATE/DELETE は SQLite トリガーで物理的に禁止
