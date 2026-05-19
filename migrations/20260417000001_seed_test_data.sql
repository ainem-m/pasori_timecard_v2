-- Seed test data for development
-- カード ID '01010112A91B9843' を持つ従業員を登録

INSERT INTO employee (
    id, display_name, employment_type, affiliation, is_active, created_at, updated_at
) VALUES (
    '0195085e-9900-7f21-88f5-66778899aabb', -- UUID v7
    'テスト 太郎',
    'regular',
    '開発部',
    1,
    '2026-04-17T00:00:00+09:00',
    '2026-04-17T00:00:00+09:00'
);

INSERT INTO card (
    id, employee_id, card_identifier, card_label, is_active, created_at, updated_at
) VALUES (
    '0195085e-9901-7acc-99aa-bbccddeeff00',
    '0195085e-9900-7f21-88f5-66778899aabb',
    '01010112A91B9843', -- スキャンされた IDm
    'テスト用カード',
    1,
    '2026-04-17T00:00:00+09:00',
    '2026-04-17T00:00:00+09:00'
);
