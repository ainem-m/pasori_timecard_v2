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

`punch_event.occurred_at` は実打刻時刻を保存する。丸め、休憩控除、有給日数、
帳票上の `残業` などは raw 打刻から算出する derived value であり、
`punch_event` を上書きしない。

MVP の勤怠集計は `PolicyProfile` preset によって切り替える。雇用区分は
policy profile の既定割当キーであり、雇用区分そのものに計算ロジックを埋め込まない。

## 勤怠表のビュー

### AttendanceDay (1 日分)

- `date`: 対象日
- `events`: その日の `PunchEvent` の配列 (時系列順)
- `work_minutes`: 就業時間 (分単位)
- `derived`: policy profile による補助集計値 (丸め後勤務時間、有給日数、帳票用分類など)
- `has_inconsistency`: 不整合フラグ (後述)
- `status`: `unconfirmed` / `confirmed` / `locked` (締め済み)

`status` は承認/締め状態を表す。出勤、欠勤、半休、遅刻、早退などの勤務状態は
将来 `attendance_mark` として別概念で追加する。MVP では `status` に勤務状態を混ぜない。

### MonthlyTimesheet (1 ヶ月分)

- `employee_id`
- `year_month`: 対象期間
- `days`: `AttendanceDay` の配列
- `total_work_minutes`
- `derived_totals`: policy profile による月次補助集計値
- `cutoff_rule`: 締めルール (例: 15日締め / 月末締め)
- `period_start`, `period_end`: 実際の期間 (例: 前月 16 日 〜 当月 15 日)
- `policy_profile`: 適用した policy preset 名

## 締め日

- 設定可能、**既定は 15 日**
- 指定方法は **固定日 (`1..=28`)** または **月末**
- 締め日は Server 側 `settings` テーブルで管理 (全体設定)
- 締め日変更時は即時反映 (過去データの再計算は発生しない、期間の区切り方だけが変わる)

### 締め日計算の例

締め日 = 15 の場合:
- 2026-04 の期間 = 2026-03-16 〜 2026-04-15
- 2026-05 の期間 = 2026-04-16 〜 2026-05-15

月末締めの場合:
- 2026-04 の期間 = 2026-04-01 〜 2026-04-30
- 2026-02 の期間 = 2026-02-01 〜 2026-02-28 (うるう年なら 29)

### 制約

- 固定日指定は `1..=28` のみ許可する
- `29..=31` は「固定日」ではなく、「月末」を選ぶことで表現する
- 期間開始日は「前回締め日の翌日」、期間終了日は「今回締め日」

## 勤務時間の計算

### MVP 実装

- 同日の ClockIn 〜 ClockOut をペアにして差分 (分単位) を合算
- raw 打刻は丸めない。丸めは policy profile の derived value にのみ適用する
- 基本の `work_minutes` は raw 打刻ベースの参考値とし、給与・帳票用の値は policy profile ごとに算出する
- 打刻種別が ClockIn, ClockOut 以外 (将来拡張) は現時点では集計対象外

### PolicyProfile preset

MVP では以下 3 つの preset を持つ。詳細な判断理由は ADR 0015 / ADR 0016 に従う。

#### `legacy_regular_2026`

対象:

- 正社員

前提:

- 院内では 1 年単位の変形労働時間制を前提に紙の年間カレンダー、紙シフト表、
  紙の給与最終計算を併用する
- v2 MVP は年間変形制の完全判定を行わず、紙計算の補助集計を提供する
- 年間カレンダーは 3 月 16 日から翌 3 月 15 日
- 月次勤怠は 15 日締め

通常勤務:

- `08:30-12:55`
- `14:00-16:00`
- `16:15-19:00`
- 所定終業時刻は `19:00`

derived value:

- `fixed_time_extra_minutes`: 平日 `19:00` 以降の勤務を帳票上 `残業` として集計する
- `paid_leave_days`: `有給 = 1 日`, `AM有給 = 0.5 日`, `PM有給 = 0.50 日`
- `attendance_notes`: `振替`, `AM振替`, `PM振替` は表示するが、MVP では pair 管理しない

半日有給の参照時間帯:

- `AM有給`: `08:30-13:00`
- `PM有給`: `14:00-19:00`

非目的:

- 年間総労働時間の完全自動整合
- 振替勤務と振替休みの 1 対 1 管理 UI
- 振替差分の分単位精算
- 年間変形制の法令適合判定

#### `legacy_part_time_2026`

対象:

- パート

丸め:

- 出勤時刻は 30 分単位で切り上げる
- 退勤時刻は丸めない
- raw 打刻は変更しない

勤務区間:

- `ClockIn` / `ClockOut` の valid pairs を合算する
- 休憩時は `退勤 -> 出勤` で打刻する
- 通常休憩は 1 回で、現行帳票の 2 ペア表示に収まる前提

derived value:

- `counted_work_minutes`: 出勤丸め適用後の勤務時間
- `within_8h_work_minutes`: 8 時間以内の勤務時間
- `over_8h_work_minutes`: 8 時間を超えた勤務時間
- `paid_leave_days`: `有給 = 1 日`, `AM有給 = 0.50 日`, `PM有給 = 0.50 日`

備考:

- 有給系だけ給与補助集計に効く
- `追加残業` は MVP では使わない
- その他備考は表示・確認用

#### `legacy_doctor_2026`

対象:

- ドクター

主集計:

- 出勤日数
- 有給日数

derived value:

- `work_days`: 出勤日数
- `paid_leave_days`: `有給 = 1 日`, `AM有給 = 0.50 日`, `PM有給 = 0.50 日`
- `reference_work_minutes`: 勤務時間の参考表示。給与値にはしない

非目的:

- ドクターの残業主集計
- ドクターの勤務時間を給与値として扱うこと

### 備考欄

MVP で構造化対象とする備考は以下に絞る。

- `有給`
- `AM有給`
- `PM有給`
- `振替`
- `AM振替`
- `PM振替`

その他備考は表示・確認用であり、給与補助集計には入れない。

### 丸め

- `RoundingPolicy` trait (`core::rounding`) で切り替え可能
- raw 打刻保存の既定は常に `NoRounding` (素通し)
- policy profile による帳票/集計用丸めは derived value として適用する
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
- CSV: 従業員 × 日付 × 打刻時刻 + policy profile ごとの補助集計値
- Excel: 同上 + 書式 (締め日区切り、不整合ハイライト、院内帳票互換列)
- ただし内部の**正本は DB**。CSV/Excel は出力専用でインポート機能はない
- CSV/Excel は給与計算エンジンではなく、紙計算・確認用の補助資料である

## MVP 非目的

- 給与計算エンジン
- 年間変形労働時間制の法令適合判定
- 年間総労働時間の完全自動整合
- 振替勤務と振替休みの 1 対 1 管理 UI
- 振替差分の分単位精算
- `追加残業` の自動集計
- 複数休憩への帳票完全対応
- 法定休憩不足の自動確定判定

## TDD 対象

- 締め日計算 (proptest: 任意の year_month, cutoff_rule に対し、期間が 1 ヶ月 ±1 日以内)
- ペアリング (proptest: 任意の打刻列に対し、不整合フラグが想定通り)
- 勤務時間集計
- raw 打刻が policy 丸めで変更されないこと
- `legacy_regular_2026`: `19:00` 以降だけ `fixed_time_extra` に入ること
- `legacy_part_time_2026`: 出勤 30 分切り上げ、退勤丸めなし、8 時間以内/8 時間超の分離
- `legacy_doctor_2026`: 出勤日数・有給日数が主集計で、勤務時間は参考表示に留まること
