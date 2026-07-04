-- audit_log を hash chain 化するための列を追加する。
--
-- 既存 DB では migration 前の audit_log は hash を持たない legacy entry として残る。
-- migration 後にアプリケーション経由で追記される entry から chain を開始する。

ALTER TABLE audit_log ADD COLUMN prev_hash TEXT;
ALTER TABLE audit_log ADD COLUMN entry_hash TEXT;

CREATE UNIQUE INDEX idx_audit_entry_hash
ON audit_log(entry_hash)
WHERE entry_hash IS NOT NULL;
