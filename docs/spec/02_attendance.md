# 詳細仕様: 勤怠表・集計・修正

## スコープ

- 勤怠表 (日次 / 月次) の表示仕様
- 締め日の扱い
- 勤務時間の計算
- 打刻修正
- CSV / Excel エクスポート (補助機能)

## データモデル

正本は `punch_event` テーブル。ビューとして `AttendanceDay` / `MonthlyTimesheet`
をアプリケーション層で組み立てる (永続化はしない)。

## 勤怠表のビュー

### AttendanceDay (1 日分)

- `date`: 対象日
- `events`: その日の `PunchEvent` の配列 (時系列順)
- `work_minutes`: 就業時間 (分単位)
- `has_inconsistency`: 不整合フラグ (後述)
- `status`: `unconfirmed` / `confirmed` / `locked` (締め済み)

### MonthlyTimesheet (1 ヶ月分)

- `employee_id`
- `year_month`: 対象期間
- `days`: `AttendanceDay` の配列
- `total_work_minutes`
- `cutoff_date`: 締め日 (例: 15)
- `period_start`, `period_end`: 実際の期間 (例: 前月 16 日 〜 当月 15 日)

## 締め日

- 設定可能、**既定は 15 日**
- 締め日は Server 側 `settings` テーブルで管理 (全体設定)
- 締め日変更時は即時反映 (過去データの再計算は発生しない、期間の区切り方だけが変わる)

### 締め日計算の例

締め日 = 15 の場合:
- 2026-04 の期間 = 2026-03-16 〜 2026-04-15
- 2026-05 の期間 = 2026-04-16 〜 2026-05-15

## 勤務時間の計算

### MVP 実装

- 同日の ClockIn 〜 ClockOut をペアにして差分 (分単位) を合算
- 休憩控除は行わない (MVP では `default_break_minutes` の概念のみ、減算しない)
- 打刻種別が ClockIn, ClockOut 以外 (将来拡張) は現時点では集計対象外

### 丸め

- `RoundingPolicy` trait (`core::rounding`) で切り替え可能
- MVP 実装は `NoRounding` (素通し)
- 将来追加: 出勤のみ切り上げ、退勤は 1 分単位、などのポリシー

### 不整合フラグ (has_inconsistency)

以下のケースで true となる:

- ClockIn のみで ClockOut がない日 (退勤漏れ疑い)
- ClockOut のみで ClockIn がない日 (出勤漏れ疑い)
- 同日に複数ペアがあって間隔が異常 (例: 3 時間未満の入退勤ループ)
- 24 時間超の継続勤務

UI では ⚠️ マークで表示。

## 打刻修正

### 種類

- **作成**: 打刻そのものがない日を管理者が追加する
- **編集**: 既存打刻の時刻 / 種別を変更する
- **削除**: ソフトデリート (`deleted_at` をセット、audit に残す)

### 必須入力

- 修正理由 (`correction_reason`, 必須)
- 修正者 (自動的に `actor_id` から記録)

### 監査

すべての修正は `audit_log` に append される:

- `action`: `punch.create_manual` / `punch.update` / `punch.soft_delete`
- `target_type`: `punch_event`
- `target_id`: 対象 punch_event の UUID
- `before_json`: 変更前 (削除・更新時)
- `after_json`: 変更後 (作成・更新時)
- `reason`: `correction_reason`

### 本人修正 vs 管理者修正

- 本人修正: LINE WORKS 経由、当日中の軽微修正のみ自動承認。それ以外は requested 状態で保留
- 管理者修正: Admin Web から制約なく可能 (ただし audit に残る)

## 締め済み期間の修正

- 設定で「締め日の N 日後 (例: 締め日 15 日 + 5 日猶予 = 20 日) 以降は lock」とする
- Lock 済み期間の修正は LINE WORKS 経由では自動却下
- 管理者 Web UI では警告ダイアログ後に強行可 (audit 必須)

## CSV / Excel エクスポート

- Admin Web の画面に「月次エクスポート」ボタン
- CSV: 従業員 × 日付 × 打刻時刻 + 勤務時間合計
- Excel: 同上 + 書式 (締め日区切り、不整合ハイライト)
- ただし内部の**正本は DB**。CSV/Excel は出力専用でインポート機能はない

## TDD 対象

- 締め日計算 (proptest: 任意の year_month, cutoff_date に対し、期間が 1 ヶ月 ±1 日以内)
- ペアリング (proptest: 任意の打刻列に対し、不整合フラグが想定通り)
- 勤務時間集計
