# 詳細仕様: シフト管理

## スコープ

- シフト表の作成・編集・公開
- シフト種別マスタ
- シフトと実績の差分検出

## 方針

MVP では **人が作り、人が修正し、人が確認できるシフト表管理** を成立させる。
自動シフト作成エンジンは post-MVP。

## データモデル

### ShiftType (マスタ)

| フィールド | 型 | 備考 |
|---|---|---|
| id | UUID v7 | |
| code | String | 例: "NORMAL", "AM", "PM", "OFF", "PAID", "SPECIAL", "STANDBY" |
| display_name | String | 例: "通常勤務", "午前", "午後", "休み", "有休", "特別休暇", "予備枠" |
| planned_start_time | Option\<Time\> | |
| planned_end_time | Option\<Time\> | |
| default_break_minutes | Option\<u32\> | |
| color | String | hex (#RRGGBB) |
| is_active | bool | |

初期シードデータとして上記 7 種類を投入する。

### ShiftAssignment

| フィールド | 型 | 備考 |
|---|---|---|
| id | UUID v7 | |
| employee_id | UUID | |
| date | jiff::civil::Date | |
| shift_type_id | UUID | |
| planned_start_at | Option\<Zoned\> | shift_type のデフォルトを override |
| planned_end_at | Option\<Zoned\> | 同上 |
| note | String | |
| status | enum | `draft` / `published` / `finalized` |
| created_at | Zoned | |
| updated_at | Zoned | |

(employee_id, date) は UNIQUE 制約。

### 監査ログ

MVP ではシフト変更の専用ログテーブルは作らない。
シフトの作成・更新・削除・公開は `audit_log` に記録する。

## ステータス遷移

```
draft ──[admin が公開]──▶ published ──[締め日到来]──▶ finalized
   │                          │
   └─[admin が削除]─          └─[admin が差し戻し]─▶ draft
```

- `draft`: 作成中。従業員には見えない
- `published`: 公開済み。従業員が LINE WORKS で照会可能
- `finalized`: 締め済み。LINE WORKS の自動修正は不可

## UI

### Admin Web

- 月カレンダービュー (月単位)
- 従業員行 × 日付列のテーブルビュー
- ドラッグ & ドロップでシフト種別を割り当て
- 一括編集 (選択範囲に同じ種別を適用)
- 「公開」ボタン (一括 draft → published)

### Terminal

Terminal には **シフト表示機能は不要**。必要なら Admin Web で別途モニタする。

## シフト vs 実績の差分検出

### 検出すべき差分

以下を「要確認」として Admin Web の日次チェック画面 / 月次画面で表示する。

1. **出勤予定 + 打刻なし**: 無断欠勤の疑い
2. **シフト外打刻**: 予定なしの日に打刻あり
3. **大幅な時刻ずれ**: 予定 09:00 に対して実績 10:30 など (閾値設定可能、既定 30 分)
4. **退勤打刻漏れ**: 出勤打刻のみでその日の退勤がない
5. **休暇予定日の打刻**: 有休 / 休み予定の日に打刻あり

### 実装

`core::application::attendance::compare_with_shift` として純関数で実装。
戻り値は `ShiftMismatch` の配列とし、`MissingPunch` / `PunchOnScheduledOffDay` /
`StartTimeMismatch` / `EndTimeMismatch` / `ClockOutMissing` を扱う。
proptest で性質検証可能にする。

## 通知

差分検出結果は LINE WORKS 通知の候補。MVP 時点では以下を送る:

- 打刻漏れ疑い (Employee 本人 + 管理者)
- シフト外打刻 (管理者)
- 大幅時刻ずれ (管理者)

通知送信は `Notifier::MissingPunchSuspected` 等で発火。

## Excel インポート / エクスポート

Excel でのシフト表管理を置き換えるのが目的だが、移行期間用に以下を提供する。

- **エクスポート**: 月単位で xlsx 出力 (従業員 × 日付 × シフト種別)
- **インポート**: xlsx を読み込んで draft として import (post-MVP、v1.1 以降)

## ログ方針

- MVP では `ShiftChangeLog` は導入しない
- post-MVP で必要性が出た場合に専用ログを再検討する

## TDD 対象

- シフト vs 実績差分検出 (proptest)
- ステータス遷移 (不正遷移を弾く)
- 月次シフト集計
