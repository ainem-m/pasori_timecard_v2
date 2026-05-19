ALTER TABLE admin_user
ADD COLUMN failed_login_attempts INTEGER NOT NULL DEFAULT 0;

ALTER TABLE admin_user
ADD COLUMN locked_until TEXT;

ALTER TABLE admin_session
ADD COLUMN last_active_at TEXT;

UPDATE admin_session
SET last_active_at = created_at
WHERE last_active_at IS NULL;
