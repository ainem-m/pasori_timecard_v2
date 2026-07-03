# OSS / 公開仕様に見る勤怠 policy パターン調査

- 日付: 2026-05-20
- ブランチ: `codex/timesheet-policy-research`
- 目的: 「よくある policy」を推測で決めず、OSS と公開仕様から勤怠計算の概念パターンを抽出する。
- スコープ: docs-only 調査。採用する policy は後続 ADR で決める。

## TODO

- [x] OSS / 公開仕様の候補を洗い出す。
- [x] 各製品の勤怠・休憩・残業・丸め・申請の概念を抽出する。
- [x] 共通パターンを v2 の policy 候補に変換する。
- [x] MVP に入れるべきもの、後回しにすべきものを分ける。
- [ ] 日本国内運用・院内テンプレートと照合して ADR 化する。

## 調査対象

| 製品/プロジェクト | 種別 | 見た理由 |
|---|---|---|
| TimeTrex | オープンソース版あり + 公開管理者ドキュメント | policy/rule-based の粒度が細かい。丸め、休憩、残業、premium、schedule policy が分離されている。 |
| Frappe HR / ERPNext HR | OSS + 公開ドキュメント | shift type、auto attendance、attendance request、leave policy の分解が参考になる。 |
| Odoo Attendances | 公開ドキュメント + 一部 OSS エコシステム | kiosk 打刻、extra hours、tolerance、manager approval の考え方が参考になる。 |
| Kimai | OSS + 公開ドキュメント | attendance 専用ではなく time tracking 寄り。丸め・重複・tracking mode・plugin の境界が参考になる。 |
| OrangeHRM | OSS starter + 公開ヘルプ/製品資料 | attendance、pay policy、overtime、rounding、bulk approval のパターン確認用。 |

注記:

- TimeTrex は OSS 版があるが、参照した細かな policy ドキュメントは enterprise manual に含まれる。
- Kimai は勤怠給与計算より project time tracking に寄っているため、労務 policy の参考としては限定的。
- Odoo はバージョンや edition により機能差がある。ここでは公開ドキュメント上の概念だけを見る。

## 製品別の抽出

### TimeTrex

TimeTrex はかなり明示的に「policy を合成する」設計になっている。

抽出した概念:

| 概念 | 内容 | v2 への示唆 |
|---|---|---|
| Policy Group | 従業員に適用する policy の束 | `employment_type` だけでなく `policy_profile` が必要になりうる。 |
| Schedule Policy | 特定シフトに meal/break/overtime/premium/undertime を上書き適用 | 雇用区分の既定 policy と日別シフト policy の override が必要。 |
| Rounding Policy | punch type ごとに interval/direction/grace/strict schedule を指定 | `RoundingPolicy` は「出勤だけ」では足りず、対象 punch type と strict schedule を持つ可能性がある。 |
| Meal Policy | 食事休憩の Normal / Auto-Deduct / Auto-Add、Active After、Meal Time | 休憩は「打刻で控除」「自動控除」「有給休憩加算」を分ける必要がある。 |
| Break Policy | Meal とほぼ同型。複数 break、Active After、Auto-Deduct/Add | 休憩 policy は 1 個ではなく、複数段階を許す設計が必要。 |
| Overtime Policy | Daily/Weekly/Bi-weekly/Holiday/Over Schedule/Consecutive Days 等 | overtime は単一 enum ではなく「計算期間」と「発火条件」を持つ。 |
| Regular Time Policy | worked time を pay code に分類。複数 regular policy は排他的 | 「通常勤務」も分類 policy。残業だけを特別扱いしない。 |
| Premium Policy | 夜間、週末、休日、危険作業、callback、minimum shift 等。非排他的 | 深夜/休日/職務手当は overtime と独立して重なる。 |
| Undertime Absence | scheduled time より短い勤務を absence/docking/accrual として扱う | 正社員の早退/不足時間は warning だけでなく absence 分類になりうる。 |
| Start/Stop Window | punch をどの予定シフトに関連付けるかの探索幅 | split shift と日跨ぎには shift matching policy が必要。 |

特に重要な観察:

- 「休憩」「残業」「premium」「不足勤務」は別 policy で、Schedule Policy がそれらを shift 単位に override する。
- 丸めは raw punch を保持しつつ、計算・表示側に適用する考え方が強い。
- premium は overtime と違い非排他的。たとえば週末 premium と夜間 premium は同じ勤務時間に重なりうる。

### Frappe HR / ERPNext HR

Frappe HR は shift を中心に auto attendance を組み立てている。

抽出した概念:

| 概念 | 内容 | v2 への示唆 |
|---|---|---|
| Shift Type | Start Time、End Time、Holiday List、Auto Attendance | `shift_type` は単なる予定表示ではなく、勤怠計算の基準になる。 |
| Night Shift | End Time < Start Time の場合は翌日終了の night shift とみなす | 日跨ぎ勤務の first-class support が必要。 |
| Check-in interpretation | 交互に IN/OUT とみなす、または Log Type を厳密に使う | PaSoRi の推定打刻と、明示打刻種別の両方を扱える必要がある。 |
| Working Hours Calculation | First Check-in/Last Check-out、または Every Valid Check-in/Check-out | 休憩を含める集計と、外出/休憩を除く集計を選べる。 |
| Shift window | shift start 前の check-in 許容、shift end 後の check-out 許容 | 早出/残業をシフトに紐づける window が必要。 |
| Holiday behavior | holiday では auto attendance をスキップ。設定で holiday checkin も処理 | 休日勤務は「無視」「処理」「警告/申請要」に分かれる。 |
| Half Day / Absent threshold | working hours が閾値未満なら Half Day / Absent | 勤務時間から日次状態を導く threshold policy がある。 |
| Attendance Request | 未打刻・在宅・外勤などの attendance regularization。既存 attendance を overwrite 可能 | v2 の LINE WORKS 申請/管理者修正に近い。承認 workflow と audit が必要。 |
| Leave Policy | leave type ごとの annual allocation を policy として割当 | 有給取得集計と有給付与/残数管理は別モジュール。 |

特に重要な観察:

- 打刻列から勤務時間を作るとき、First/Last 方式と valid pairs 方式の両方が必要。
- Shift Assignment が Default Shift より優先される。v2 でも「個別日付の予定」が「雇用区分既定」を上書きする構造が自然。
- Attendance Request は correction だけでなく「在宅/外勤で打刻されない日を勤怠として認める」用途を含む。

### Odoo Attendances

Odoo は time clock/kiosk を中心に、extra hours と tolerance を持つ。

抽出した概念:

| 概念 | 内容 | v2 への示唆 |
|---|---|---|
| Kiosk / direct check-in | 専用端末または DB 上で check-in/out | PaSoRi terminal と Admin correction の分離に近い。 |
| Automatic Check-Out | 所定終了後、tolerance 経過で自動 checkout | 退勤漏れに対して「警告」だけでなく「自動補完」policy がありうる。MVP では慎重。 |
| Absence Management | time off request に紐づかない absence を attendance report に出す | 欠勤/無断欠勤/申請済休暇を分ける必要がある。 |
| Extra Hours | expected working schedule を超える時間 | 残業は schedule と結びつく。 |
| Tolerance in favor of company | 会社側猶予。閾値未満の超過時間を overtime にしない | overtime threshold/grace policy。 |
| Tolerance in favor of employee | 従業員側猶予。少し短い勤務を不足扱いにしない | late/early/undertime grace policy。 |
| Extra Hours Validation | 自動承認または manager approval | 残業を計算するだけでなく、承認済みかどうかを別状態にする。 |
| Display Extra Hours | kiosk/check-out 時に extra hours を表示 | UI feedback policy。 |

特に重要な観察:

- 残業計算には「会社側に有利な tolerance」と「従業員側に有利な tolerance」が分かれている。
- 残業を自動承認するか、管理者承認にするかが設定になる。
- 退勤漏れ自動補完は便利だが、監査性と本人確認が必要。

### Kimai

Kimai は勤怠よりも project timesheet / invoicing 寄り。
ただし、汎用 time tracking の設計として参考になる。

抽出した概念:

| 概念 | 内容 | v2 への示唆 |
|---|---|---|
| Time-tracking mode | Default / Time-clock / Duration | UI で入力できる粒度を policy として変える。 |
| Future entries | 未来の timesheet 記録を許可するか | 勤怠では未来打刻は禁止、予定/シフトは別モデル。 |
| Empty duration | duration 0 を許可するか | 打刻漏れ/未完了勤務の扱い。 |
| Overlapping entries | 重複時間を許可するか | 複数勤務/複数業務/休憩との関係。勤怠では原則 warning。 |
| Simultaneous running entries | 同時進行 record 数 | 複数 job/task には必要だが、勤怠打刻では原則 1。 |
| Maximum duration | 1 record の最大時間 | 24h 超勤務・退勤漏れの guard。 |
| Break tracking | break time tracking | 休憩は feature flag になりうる。 |
| Rounding | start/end/duration の丸め単位、曜日適用、mode default/closest/floor/ceil | v2 の丸めも start/end/duration 別、曜日条件を将来考慮。 |
| Plugins | plugin support | ただし Kimai は正本勤怠ではなく project time tracking なので、v2 では plugin の権限を絞るべき。 |

特に重要な観察:

- time-clock mode は「一般ユーザーは時刻編集できないが、管理者は補正できる」という権限分離を持つ。
- 丸め対象が start/end/duration で分離されている。v2 の現在の `RoundingPolicy.round(event_type, at)` だけでは duration rounding を表現しにくい。

### OrangeHRM

公開情報上、Open Source Starter と有償機能に分かれるが、勤怠概念の確認には有用。

抽出した概念:

| 概念 | 内容 | v2 への示唆 |
|---|---|---|
| Attendance sheet | holidays、weekends、leave、no attendance records を一覧で可視化 | 月次表には打刻だけでなく休日/休暇/未打刻状態を重ねる必要がある。 |
| Pay policies | overtime、double-time、weekly、consecutive rules、rounding options | TimeTrex と同様、残業は日次/週次/連続勤務/二倍時間など複数軸。 |
| Round off to shift times | 5/10/15/30/60 分など | shift-based rounding preset。 |
| Bulk approval | employee attendance sheet の一括承認 | 月次締め/確認 workflow の候補。 |

特に重要な観察:

- 月次勤怠は「打刻表」ではなく、休日・休暇・欠勤・承認状態を重ねた review sheet になる。
- 連続勤務 rules は日本でも将来の warning/premium policy として意味がある。

## 横断的に見えた policy パターン

### 1. Policy Profile / Policy Group

雇用区分に直接ロジックを埋め込むのではなく、従業員に policy の束を割り当てる。

```text
Employee -> PolicyProfile
PolicyProfile -> PeriodPolicy
PolicyProfile -> RoundingPolicy
PolicyProfile -> BreakPolicy
PolicyProfile -> OvertimePolicy[]
PolicyProfile -> WarningPolicy[]
```

理由:

- 正社員でも部署や個人でルールが違うことがある。
- パートでも固定勤務とシフト勤務がある。
- 医療機関では職種と給与/勤怠ルールが一致しない可能性がある。

### 2. Schedule Override

既定 policy を持ちつつ、日別シフトが一部 policy を上書きする。

```text
PolicyProfile default:
  break = 60m auto-deduct after 6h

Specific shift:
  break = 30m auto-deduct after 5h
```

TimeTrex の Schedule Policy、Frappe HR の Shift Assignment がこの型。

### 3. Raw Punch と Derived Time の分離

raw punch は実打刻として残す。
丸め、休憩控除、自動補完、残業分類は derived result。

v2 で守るべき境界:

- `punch_event.occurred_at`: 実時刻。
- `attendance_segment.raw_start/raw_end`: 実時刻ベース。
- `attendance_segment.counted_start/counted_end`: policy 適用後。
- `timesheet_day.warning[]`: policy 違反/不整合。
- `audit_log`: 補正・自動補完・承認の根拠。

### 4. Punch Interpretation

打刻ログから IN/OUT を決める policy。

| パターン | 内容 |
|---|---|
| explicit | 端末が IN/OUT を明示する |
| alternating | その日の/その shift の 1 個目を IN、2 個目を OUT と交互解釈 |
| inferred | 直近履歴から次種別を推定 |
| manual correction | 管理者/本人申請で補正 |

v2 の `DefaultPunchPolicy` は `inferred`。Frappe HR は `explicit` と `alternating` を持つ。

### 5. Working Hours Calculation

| パターン | 内容 | 向く職場 |
|---|---|---|
| first_last | 最初の IN から最後の OUT まで。途中外出/休憩を含める | 単純な在席時間 |
| valid_pairs | IN/OUT ペアごとの合算。途中 OUT は除く | 休憩/外出打刻あり |
| scheduled | 予定勤務時間を基準にし、実打刻との差分を警告/残業にする | 正社員/シフト |
| days_only | 日数・出勤事実のみ | ドクター等の帳票 |

現在の v1 は `valid_pairs` 寄り。ただし休憩打刻はない。

### 6. Rounding

| パターン | 内容 |
|---|---|
| punch rounding | IN/OUT/休憩開始/休憩終了を個別に丸める |
| duration rounding | 勤務時間合計を丸める |
| day total rounding | 日合計が丸め単位になるよう終端を調整 |
| schedule strict rounding | 予定時刻を超える早出/残業を認めず、予定時刻へ丸める |
| grace period | 猶予内なら予定時刻または丸め境界へ寄せる |
| day-of-week condition | 曜日ごとに丸め適用を変える |

v1 院内テンプレートは `clock_in ceil 30m`、`clock_out none` の punch rounding。

### 7. Break / Meal

| パターン | 内容 |
|---|---|
| punched break | 休憩開始/終了打刻を控除 |
| auto-deduct | 一定時間以上働いたら自動控除 |
| auto-add | 有給休憩として一定時間を加算 |
| expected window | この時間帯の OUT/IN は休憩とみなす |
| min/max break detection | 一定長の OUT/IN を休憩とみなす |
| multiple breaks | 複数休憩を合算 |
| legal warning | 法定休憩不足を warning |

自動控除と法定休憩 warning は別物として扱うべき。

### 8. Overtime / Extra Hours

| パターン | 内容 |
|---|---|
| daily threshold | 1 日 N 時間超 |
| weekly threshold | 週 N 時間超 |
| bi-weekly / multi-week | 複数週単位 |
| over schedule daily | 日別予定時間超 |
| over schedule weekly | 週予定時間超 |
| fixed time window | 19:00 以降、22:00-05:00 など |
| non-working day | 非勤務日/休日の勤務 |
| consecutive days | 連続勤務日数で発火 |
| manager-approved only | 承認済みだけ残業扱い |
| tolerance | 会社側/従業員側の猶予 |

v1 院内 `正社員` は `fixed time window` の一種。
法律上の時間外とは別名にする必要がある。

### 9. Premium / Differential

TimeTrex の Premium Policy が最も分かりやすい。
overtime と premium は分ける。

| パターン | 内容 |
|---|---|
| night premium | 特定時間帯 |
| weekend premium | 特定曜日 |
| holiday premium | 休日 |
| branch/department/job differential | 部署/業務別手当 |
| callback | 一度退勤後、短時間内に呼び戻し |
| minimum shift time | 短時間出勤でも最低 N 時間扱い |
| split shift premium | シフト間隔が短い/長い場合の手当 |

MVP では給与計算をしないため、premium は warning/export 分類に留める。

### 10. Attendance Status Threshold

Frappe HR のように、working hours から日次状態を決める。

| 状態 | 例 |
|---|---|
| present | 閾値以上勤務 |
| half_day | half day threshold 未満 |
| absent | absent threshold 未満、または打刻なし |
| late | scheduled start より遅い |
| early_exit | scheduled end より早い |
| missing_punch | IN/OUT 不整合 |

v2 の `AttendanceDay.status` は現在 `unconfirmed/confirmed/locked` だが、これは承認状態。
勤務状態とは別フィールドに分ける必要がある。

### 11. Request / Regularization

| パターン | 内容 |
|---|---|
| punch correction | 打刻時刻/種別の修正 |
| attendance request | 外勤/在宅/打刻不能日の勤怠化 |
| overtime approval | extra hours を承認する |
| leave application | 休暇申請 |
| bulk regularization | 月次/週次単位の一括補正 |

v2 の LINE WORKS `attendance_request` は punch correction だけでなく、attendance request / overtime approval に拡張しうる。

### 12. Recalculation

policy や schedule を変えたときに、過去 timesheet を再計算する必要がある。

重要な分離:

- raw punch は変えない。
- policy version を記録する。
- 月次締め後は lock し、再計算には audit を残す。
- 「現在の policy で再計算した参考値」と「締め時点で確定した値」を分ける可能性がある。

## v2 への候補モデル

### 最小限ほしい用語

| 用語 | 役割 |
|---|---|
| `PolicyProfile` | 従業員または雇用区分に割り当てる policy 束 |
| `PolicyVersion` | policy 変更の履歴・締め済み再現性 |
| `ShiftPolicyOverride` | 日別シフトで既定 policy を上書き |
| `PunchInterpretationPolicy` | raw punch を IN/OUT/休憩/外出に解釈 |
| `WorkSegmentPolicy` | first_last / valid_pairs / scheduled など |
| `RoundingPolicy` | punch/duration/day total rounding |
| `BreakPolicy` | punched / auto-deduct / auto-add / warning |
| `OvertimePolicy` | daily/weekly/schedule/fixed/approved/tolerance |
| `PremiumPolicy` | 夜間/休日/職務/最低保証など |
| `AttendanceStatusPolicy` | present/half_day/absent/late/early_exit |
| `RegularizationPolicy` | 申請/承認/修正の扱い |

### MVP に入れる価値が高いもの

1. `PolicyProfile`
2. `PunchInterpretationPolicy`
3. `WorkSegmentPolicy`
4. `ReportRoundingPolicy`
5. `BreakPolicy` の `none` / `punched` / `auto_deduct` の型だけ
6. `OvertimePolicy` の `over_daily_hours` / `after_fixed_time` / `over_schedule` の型だけ
7. `AttendanceStatusPolicy` の warning 中心実装
8. `RegularizationPolicy` は既存 `attendance_request` と audit の延長

### MVP では持たない方がよいもの

- premium の給与計算。
- callback、split shift premium、minimum shift pay。
- 変形労働時間制、フレックス清算。
- overtime rate combination。
- 任意スクリプトによる DB 更新。
- policy graph の完全な UI 条件ビルダー。

## 重要な設計仮説

1. 雇用区分は policy ではなく、policy profile の既定割当キーにする。
2. 「残業」は内部語として使わず、`fixed_time_extra`、`scheduled_extra`、`statutory_overtime_candidate` のように分ける。
3. `AttendanceDay.status` は承認/締め状態なので、勤務状態は `attendance_mark` など別名にする。
4. policy は versioned にする。月次締め後の再現性が必要。
5. UI は preset + パラメータ編集。複雑な policy は ADR を書いて Rust 実装として追加する。
6. 将来 plugin を入れるなら、derived result の追加列/warning/export だけ許可する。

## 参照

- TimeTrex: Policies
  - https://help.timetrex.com/latest/enterprise/Components/Policies.htm
- TimeTrex: Schedule Policies
  - https://help.timetrex.com/latest/enterprise/Components/Schedule-Policies.htm
- TimeTrex: Rounding Policies
  - https://help.timetrex.com/latest/enterprise/Components/Rounding-Policies.htm
- TimeTrex: Meal Policies
  - https://help.timetrex.com/latest/enterprise/Components/Meal-Policies.htm
- TimeTrex: Break Policies
  - https://help.timetrex.com/latest/enterprise/Components/Break-Policies.htm
- TimeTrex: Overtime Policies
  - https://help.timetrex.com/latest/enterprise/Components/Overtime-Policies.htm
- TimeTrex: Regular Time Policies
  - https://help.timetrex.com/latest/enterprise/Components/Regular-Time-Policies.htm
- TimeTrex: Premium Policies
  - https://help.timetrex.com/latest/enterprise/Components/Premium-Policies.htm
- Frappe HR: Shift Type
  - https://docs.frappe.io/hr/shift-type
- Frappe HR: Auto Attendance
  - https://docs.frappe.io/hr/auto-attendance
- Frappe HR: Attendance Request
  - https://docs.frappe.io/hr/attendance-request
- Frappe HR: Leave Policy
  - https://docs.frappe.io/hr/leave-policy
- Odoo 18: Attendances
  - https://www.odoo.com/documentation/18.0/applications/hr/attendances.html
- Kimai: Settings
  - https://www.kimai.org/documentation/configurations.html
- Kimai: GitHub repository
  - https://github.com/kimai/kimai
- OrangeHRM: Attendance module features
  - https://help.orangehrm.com/hc/en-us/articles/900005507106-New-features-in-the-Attendance-Module
