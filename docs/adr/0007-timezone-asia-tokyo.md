# ADR 0007: タイムゾーン方針 (Asia/Tokyo aware)

- **日付**: 2026-04-16
- **状態**: Accepted
- **関連**: AGENTS.md §8

## 背景

v1 は naive datetime と aware datetime が混在し、集計時に 9 時間ずれる不具合が
実際に発生していた。v2 では最初から方針を決定的に固める。

## 決定

**保存も表示もすべて Asia/Tokyo、timezone-aware** で扱う。

- Rust 側: `jiff::Zoned` (tz = "Asia/Tokyo")
- SQLite 側: TEXT (RFC 3339 形式、例: `2026-04-16T09:15:30+09:00`)
- TypeScript 側: ISO 8601 文字列で受け渡し、日付ライブラリは `date-fns` または jiff の JS ポート

### 禁止

- `jiff::civil::DateTime` (timezone-unaware) を保存に使う
- `chrono::NaiveDateTime` / `std::time::SystemTime` を保存に使う
- Rust 層で `DateTime<Utc>` を使う
- TypeScript 層で `Date` オブジェクトを直接 JSON 化する (タイムゾーン情報が落ちる)

## 代替案と却下理由

- **UTC 保存 + 表示時 JST 変換**: 一般論としては推奨だが、本システムは単一国内
  拠点で他タイムゾーンへの拡張予定なし。JST 保存の方が DB を直接見た時の
  可読性が高い。jiff は timezone-aware がデフォルトなので aware 保存の扱いが楽。
- **naive 保存**: v1 で不具合の原因になった。採用不可。

## 境界処理

- Terminal の打刻発生時刻 = Terminal の OS 時計由来 (NTP 同期必須、±10 秒)
- Server の受信時刻 = Server の OS 時計由来 (NTP 同期必須)
- オフライン打刻時は Terminal 時計を `occurred_at` にそのまま保存し、
  `server_recorded_at` は Server が受信した時刻 (=再送時の時刻)
- 両者の差分が異常 (±10 分以上) なら `audit_log` に警告

## テスト

- proptest で「任意の Zoned を DB に保存 → 読み出し → 等値比較」が成立すること
- proptest で「任意の Zoned を JSON シリアライズ → デシリアライズ → 等値比較」が成立すること
- 夏時間切替日 (Asia/Tokyo はない) ではなく、新年 / 月跨ぎの境界ケースをテストで担保

## 結論

Asia/Tokyo で一貫、jiff 全面採用、naive 禁止。
