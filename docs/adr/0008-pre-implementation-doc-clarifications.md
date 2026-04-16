# ADR 0008: 実装前の文書整合性修正

- **日付**: 2026-04-16
- **状態**: Accepted
- **関連**: AGENTS.md, ADR 0006, ADR 0007

## 背景

実装着手前レビューで、複数の仕様文書に未整合が見つかった。
特に以下はマイグレーション、Terminal local cache、認証実装の起点になるため、
実装前に固定が必要だった。

- Terminal API token の保存方法
- `card_id` と `card_identifier` の意味
- オフライン打刻時の確認 UI と必要キャッシュ
- 監査ログの ON/OFF 可否
- `ShiftChangeLog` の扱い
- `ReaderBackend` など port の配置先
- Cloudflare Tunnel の開発/本番要件

## 決定

### Terminal API token

- Terminal API token は**暗号化して設定ファイルに保存**する
- 平文 token を設定ファイルに置くことは禁止

### `card_id` と `card_identifier`

- `card_id` は常に `card` table の主キー UUID を指す
- FeliCa IDm の hex 表現は `card_identifier` と呼ぶ

### オフライン打刻 UI

- オフライン時も online と同じ確認 UI を維持する
- そのため Terminal local cache に recent punches を保持する

### 監査ログ

- 監査ログは設定で ON/OFF 可能とする
- 既定値は全 ON とする

### シフト変更ログ

- MVP では `ShiftChangeLog` テーブルを作らない
- シフト変更は `audit_log` のみで記録する

### port 配置

- `ReaderBackend`, repository traits, `Notifier`, policy traits は
  `crates/core/src/port/` 配下にまとめる

### Cloudflare Tunnel

- 本番は独自ドメイン必須
- 開発/検証環境のみ `trycloudflare.com` を許容する

## 結論

上記で文書を更新し、以後の実装はこの方針を前提に進める。
