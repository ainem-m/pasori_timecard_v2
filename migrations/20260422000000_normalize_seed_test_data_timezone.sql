-- Normalize the development seed timestamps to an explicit Asia/Tokyo zone.
-- 既存の seed migration は適用済みチェックサム維持のため変更しない。

UPDATE employee
SET
    created_at = '2026-04-17T00:00:00+09:00[Asia/Tokyo]',
    updated_at = '2026-04-17T00:00:00+09:00[Asia/Tokyo]'
WHERE id = '0195085e-9900-7f21-88f5-66778899aabb'
  AND created_at = '2026-04-17T00:00:00+09:00'
  AND updated_at = '2026-04-17T00:00:00+09:00';

UPDATE card
SET
    created_at = '2026-04-17T00:00:00+09:00[Asia/Tokyo]',
    updated_at = '2026-04-17T00:00:00+09:00[Asia/Tokyo]'
WHERE id = '0195085e-9901-7acc-99aa-bbccddeeff00'
  AND created_at = '2026-04-17T00:00:00+09:00'
  AND updated_at = '2026-04-17T00:00:00+09:00';
