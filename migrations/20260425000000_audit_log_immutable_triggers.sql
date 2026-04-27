-- audit_log の不変性を保証するトリガー。
-- UPDATE / DELETE を禁止し、append-only を強制する。
-- 仕様: docs/spec/05_audit_and_backup.md

CREATE TRIGGER IF NOT EXISTS prevent_audit_log_update
BEFORE UPDATE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only: UPDATE is prohibited');
END;

CREATE TRIGGER IF NOT EXISTS prevent_audit_log_delete
BEFORE DELETE ON audit_log
BEGIN
    SELECT RAISE(ABORT, 'audit_log is append-only: DELETE is prohibited');
END;