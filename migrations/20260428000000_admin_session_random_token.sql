CREATE TABLE admin_session_new (
    id TEXT PRIMARY KEY NOT NULL,
    admin_user_id TEXT NOT NULL REFERENCES admin_user(id),
    issued_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    last_active_at TEXT NOT NULL,
    user_agent TEXT,
    ip_address TEXT
);

INSERT INTO admin_session_new (
    id,
    admin_user_id,
    issued_at,
    expires_at,
    last_active_at,
    user_agent,
    ip_address
)
SELECT
    id,
    admin_user_id,
    created_at,
    expires_at,
    COALESCE(last_active_at, created_at),
    NULL,
    NULL
FROM admin_session;

DROP TABLE admin_session;

ALTER TABLE admin_session_new RENAME TO admin_session;

CREATE INDEX idx_session_expires ON admin_session(expires_at);
