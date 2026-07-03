# 勤怠 policy 詳細分解調査

- 日付: 2026-05-20
- ブランチ: `codex/timesheet-policy-research`
- 目的: OSS / 公開仕様の policy 概念を、v2 の設計判断に使える粒度まで分解する。
- スコープ: docs-only 調査。実装・仕様確定はしない。

## 調査の結論

勤怠 policy は「残業」「丸め」「休憩」のような単語単位では足りない。
実際には、以下の pipeline の各段に policy が存在する。

```text
raw punch
  -> punch interpretation
  -> shift matching
  -> work segment construction
  -> rounding
  -> break/meal handling
  -> attendance mark
  -> overtime/extra/premium classification
  -> request/approval
  -> lock/recalculation/export
```

この pipeline を分けないと、設定画面が巨大な条件ビルダーになり、raw 打刻・監査ログ・帳票値の境界が曖昧になる。

## 1. Raw Punch

### 入力

- employee
- timestamp
- source: NFC / manual / import / local_cached / backend
- optional log type: IN / OUT / BREAK / TRANSFER 等
- device / terminal
- location 等

### 公開仕様で見えるパターン

| パターン | 例 | v2 への示唆 |
|---|---|---|
| kiosk/direct/backend | Odoo は kiosk と backend check-in/out を分ける | Terminal 打刻と Admin 修正は source を分ける。 |
| raw を保持 | TimeTrex は丸め前の actual punch を保持する説明がある | `occurred_at` を丸め済みで上書きしない。 |
| sync boundary | Frappe HR は `Last Sync of Checkin` 以降を auto attendance の対象にする | offline sync / import には処理済み境界が必要。 |
| error record | Odoo は 24h 未退勤や 16h 超の single period を error として扱う | v2 も error/warning を raw から派生させる。 |

### v2 の設計メモ

- `punch_event` は正本。
- raw punch の timestamp は必ず Asia/Tokyo aware。
- policy が作る値は `derived` として扱う。
- manual/import/local_cached は同じ集計対象でも、監査や警告の扱いを分ける。

## 2. Punch Interpretation Policy

打刻ログから打刻種別を決める policy。

| policy | 説明 | 採用例 | v2 での優先度 |
|---|---|---|---|
| `explicit_log_type` | 端末/ログが IN/OUT を明示する | Frappe HR の strict log type | 中 |
| `alternating_within_shift` | shift 内の 1 件目を IN、2 件目を OUT と交互解釈 | Frappe HR | 高 |
| `recent_history_inferred` | 直近履歴から次種別を推定 | v2 `DefaultPunchPolicy` | 既存 |
| `button_selected` | 端末 UI で出勤/退勤/休憩を選ぶ | 多くの time clock | 後続 |
| `manual_override` | 管理者/本人申請で種別補正 | Frappe Attendance Request / v2 correction | 高 |

### 判断点

- `DefaultPunchPolicy` は打刻登録時の推定 policy。
- 月次集計時には、raw punch を再解釈する `PunchInterpretationPolicy` も必要になる可能性がある。
- 端末で「出勤/退勤」を固定登録する職場と、交互解釈する職場は両方ある。

## 3. Shift Matching Policy

raw punch をどの勤務日/shift に帰属させるか。

| policy | 入力 | 出力 | 例 |
|---|---|---|---|
| `calendar_day` | timestamp | same date | 現行 v2 の同日集計 |
| `cutoff_time_day` | timestamp, day boundary time | attendance date | 深夜 03:00 までは前日扱い |
| `scheduled_window` | punch, shift start/end, before/after window | matched shift | Frappe の begin check-in before / allow checkout after |
| `night_shift` | shift end < start | next-day end | Frappe は end < start を夜勤扱い |
| `split_shift_window` | multiple shifts, start/stop window | matched segment | TimeTrex の Start/Stop Window |

### v2 の示唆

現行仕様の「同日の ClockIn-ClockOut ペア」だけでは、夜勤・早出・遅出・複数シフトを扱えない。
ただし MVP で夜勤まで入れると大きいので、まず以下の境界を作るのが現実的。

- MVP: `calendar_day`
- 近い後続: `scheduled_window`
- 後続 ADR: `night_shift` / `cutoff_time_day`

## 4. Work Segment Construction

勤務時間をどの区間として作るか。

| policy | 説明 | 向く運用 |
|---|---|---|
| `first_last` | 最初の IN から最後の OUT まで。途中 OUT/IN も勤務に含む | 休憩打刻を勤務時間に含める、在席時間管理 |
| `valid_pairs` | IN/OUT ペアごとに合算。OUT 中は除外 | 休憩・外出打刻あり |
| `scheduled_duration` | shift の所定時間を基準にする | 正社員、みなし勤務 |
| `days_only` | 時間ではなく出勤日数を数える | ドクター型帳票 |
| `manual_attendance_mark` | 申請/管理者承認で日次状態を直接作る | 外勤、在宅、出張 |

### 公開仕様からの根拠

- Frappe HR は `First Check-in and Last Check-out` と `Every Valid Check-in and Check-out` を分けている。
- TimeTrex は punch pair を前提にしつつ、Lunch/Break/Day Total rounding では最終 punch を調整して合計値を丸める。
- Frappe Attendance Request は、打刻がない日や欠勤扱いになった日を申請で attendance 化できる。

### v2 の示唆

`build_attendance_day` は最終的に `Vec<WorkSegment>` を作る方がよい。
`work_minutes` は segment の合算結果であり、raw punch から直接 1 個の数値にしない。

```text
WorkSegment {
  raw_start
  raw_end
  counted_start
  counted_end
  segment_kind
  source_policy
}
```

## 5. Rounding Policy

TimeTrex の公開仕様を見ると、丸めは単純な `event_type -> rounded_at` では足りない。

### 軸

| 軸 | 候補 |
|---|---|
| 対象 | in, out, break_in, break_out, transfer, lunch_total, break_total, day_total |
| 方向 | up, down, nearest/average, nearest partial up/down |
| 単位 | 1, 5, 10, 15, 30, 60 分 |
| grace | 丸めが始まる前の猶予 |
| window base | scheduled time, static time, static total time, no schedule |
| strict schedule | 予定外の早出/残業を予定時刻へ寄せる |
| retroactive | policy 変更時に過去へ再適用するか |

### v2 の示唆

現行 trait:

```rust
fn round(&self, event_type: PunchEventType, at: &Zoned) -> Zoned;
```

これは MVP の `NoRounding` には十分だが、公開仕様レベルの丸めには情報が不足する。
将来は以下が必要になる。

```text
round(context):
  punch_type
  raw_at
  scheduled_start/end
  segment_raw_start/end
  day_total
  employee_policy_profile
```

重要:

- 出勤丸め、退勤丸め、日合計丸めは別 policy。
- v1 院内式は `in up 30m`、`out none`。
- 日合計丸めは最終退勤時刻を調整する実装になりがちなので、raw と counted を必ず分ける。

## 6. Break / Meal Policy

TimeTrex は Meal と Break を分けるが、構造は近い。

### パターン

| policy | 説明 | 例 |
|---|---|---|
| `none` | 休憩控除なし | 現行 v1 テンプレート |
| `punched` | break start/end または OUT/IN を休憩として控除 | 休憩打刻あり |
| `auto_deduct` | 一定時間以上働いたら固定休憩を控除 | 6h 超 45m、8h 超 60m など |
| `auto_add` | 有給休憩として勤務時間に加算 | paid break |
| `expected_window` | first punch から N 時間後の window を休憩とみなす | TimeTrex |
| `duration_detect` | OUT/IN の長さが min/max に入れば休憩とみなす | TimeTrex |
| `multiple_breaks` | 複数休憩を合算 | TimeTrex |
| `legal_warning_only` | 控除せず、法定休憩不足を warning | 日本向け MVP 候補 |

### v2 の示唆

休憩 policy は「控除」と「警告」を分ける。

```text
BreakPolicyResult:
  deducted_minutes
  paid_break_minutes
  break_segments
  warnings
```

現行 v1 は休憩控除なし。
日本の法定休憩は自動控除義務ではなく、休憩付与義務なので、MVP で勝手に控除するより warning が安全。

## 7. Attendance Mark Policy

日次の勤務状態を決める policy。
これは `AttendanceDay.status` の `unconfirmed/confirmed/locked` とは別物。

### パターン

| mark | 条件例 |
|---|---|
| `present` | working_minutes >= present threshold |
| `half_day` | half_day threshold 未満 |
| `absent` | absent threshold 未満、または check-in なし |
| `late` | in_time > scheduled_start + grace |
| `early_exit` | out_time < scheduled_end - grace |
| `holiday_work` | holiday に check-in がある |
| `missing_punch` | IN/OUT 不整合 |
| `manual_present` | attendance request / admin correction |

### 公開仕様からの根拠

- Frappe HR は working hours threshold で Half Day / Absent を決める。
- Frappe HR は Late Entry / Early Exit の grace period を持つ。
- Odoo は expected working schedule に対して extra/short を見る。

### v2 の示唆

`AttendanceDay` は少なくとも以下を分ける必要がある。

```text
approval_status: unconfirmed / confirmed / locked
attendance_mark: present / half_day / absent / holiday_work / missing_punch / ...
warnings: Vec<AttendanceWarning>
```

## 8. Overtime / Extra Hours Policy

「残業」は複数の概念が混ざるため、内部では分ける。

### 発火条件の軸

| 軸 | 例 |
|---|---|
| daily | 1 日 8h 超 |
| weekly | 週 40h 超 |
| bi-weekly / multi-week | 2 週、3-12 週 |
| schedule daily | その日の予定勤務時間超 |
| schedule weekly | 週の予定勤務時間超 |
| fixed time | 19:00 以降 |
| holiday | 休日勤務 |
| consecutive days | 連続勤務 N 日目 |
| specific day of week | 特定曜日 |
| approved only | 管理者承認済みのみ |
| tolerance | 一定分以下は無視、一定分以下の不足は免除 |

### 重複と優先順位

TimeTrex は overtime policy を複数持つが、premium と違って排他的な扱いをする。
この点が重要。

```text
regular time -> overtime bucket A -> overtime bucket B
```

同じ 30 分を daily overtime と weekly overtime に二重計上すると給与連携が壊れる。
一方で premium は同じ 30 分に night premium と weekend premium が重なることがある。

### v2 の命名案

`overtime_minutes` だけでは足りない。

```text
extra_time_buckets:
  fixed_time_extra
  scheduled_extra
  statutory_daily_overtime_candidate
  statutory_weekly_overtime_candidate
  holiday_work
```

MVP では「候補」や「参考値」として扱い、給与計算確定値にしない。

## 9. Premium / Differential Policy

Premium は「割増対象の分類」や「手当」に近く、overtime と重なる。

### パターン

| premium | 説明 |
|---|---|
| night | 22:00-05:00 など |
| weekend | 土日 |
| holiday | 休日 |
| department/job | 部署・業務別 |
| callback | 退勤後短時間で呼び戻し |
| minimum shift | 短時間勤務でも最低 N 時間扱い |
| split shift | シフト間隔による手当 |
| no_break | 一定休憩を取らなかった場合 |

### v2 の扱い

MVP では実装しない。
ただし将来の export を考えると、`PremiumPolicy` という名前だけは設計メモに残す価値がある。

## 10. Request / Approval / Regularization

打刻正本とは別に、申請で attendance を補正する層。

### パターン

| request | 内容 |
|---|---|
| punch correction | 打刻時刻/種別修正 |
| missing punch | 出勤漏れ/退勤漏れ追加 |
| attendance request | 外勤/在宅/出張などで出勤扱いにする |
| overtime approval | extra hours を承認する |
| holiday work approval | 休日出勤を承認する |
| leave request | 有給/半休/欠勤 |
| bulk regularization | 月次/週次でまとめて申請 |

### 公開仕様からの根拠

- Frappe Attendance Request は未打刻日の勤怠化、既存 attendance の overwrite、bulk request を持つ。
- Odoo は extra hours を自動承認または manager approval にできる。

### v2 の示唆

現在の `attendance_request` は LINE WORKS 経由の修正申請として始まっているが、将来は次のように type を広げられる。

```text
request_type:
  punch_correction
  missing_in
  missing_out
  attendance_regularization
  overtime_approval
  leave_request
```

ただし、打刻修正と日次 attendance mark の上書きは監査上の意味が違う。
同じテーブルに入れる場合も `target_kind` を分ける必要がある。

## 11. Recalculation / Policy Version

policy 変更後に過去の勤怠をどう扱うか。

### パターン

| パターン | 内容 |
|---|---|
| forward only | policy 変更後の新規/修正データだけに適用 |
| manual recalculation | 管理者が対象期間を指定して再計算 |
| automatic affected-days recalculation | schedule/timesheet 変更時に関連日/週を再計算 |
| lock after closing | 締め後は固定。再計算は audit 必須 |
| rounding not retroactive | 丸めだけは過去再適用しない、または個別保存が必要 |

TimeTrex は policy group 変更が過去データへ自動適用されず、retroactive には recalculation が必要という考え方を持つ。
これは v2 にも重要。

### v2 の示唆

```text
policy_profile
  id
  version
  effective_from
  effective_to

monthly_timesheet_snapshot
  policy_profile_version
  calculated_at
  locked_at
```

MVP では snapshot 永続化までは不要でも、`policy_version` の考え方を早めに ADR 化したい。

## 12. Export / Report Policy

内部集計と外部帳票は別。

### パターン

| report | 内容 |
|---|---|
| payroll export | 給与ソフト向け CSV |
| legacy spreadsheet | 既存 Excel 互換 |
| manager review | 不整合/承認待ち一覧 |
| employee confirmation | 本人確認用 |
| statutory warning | 長時間/休憩不足/休日不足 |

v1 院内テンプレートは `legacy spreadsheet`。
そこにある 30 分切り上げや 19:00 残業は、DB 正本ではなく `ReportPolicy` として扱うのが安全。

## v2 で最初に ADR 化したい論点

### ADR-A: PolicyProfile を導入するか

候補:

```text
employee.employment_type -> default_policy_profile
employee.policy_profile_id -> optional override
```

理由:

- 雇用区分と勤怠ルールは一致しないことがある。
- OSS/公開仕様は policy group/profile 的な概念を持つことが多い。

### ADR-B: AttendanceDay の状態を分離するか

現状:

```text
status: unconfirmed / confirmed / locked
```

提案:

```text
approval_status: unconfirmed / confirmed / locked
attendance_mark: present / half_day / absent / holiday_work / missing_punch / ...
```

理由:

- 承認状態と勤務状態は別概念。
- Frappe/Odoo/OrangeHRM などは late/early/absent/present を別に扱う。

### ADR-C: RoundingPolicy trait を将来拡張前提にするか

現状 trait はシンプル。
MVP は維持してよいが、将来のために「これは保存時刻ではなく report/counting 用」と明記する。

### ADR-D: 休憩を控除 policy と warning policy に分けるか

自動控除は職場運用。
法定休憩不足は warning。
この 2 つを混ぜない。

### ADR-E: 残業を `overtime` という 1 フィールドにしない

最低でも以下に分ける。

- fixed_time_extra
- scheduled_extra
- daily_over_threshold
- weekly_over_threshold
- approved_extra

MVP は `fixed_time_extra` と `over_daily_8h` の表示/集計まででよい。

## MVP policy preset の再整理

### `legacy_part_time_2026`

院内パート互換。

```text
period: cutoff day 15
punch_interpretation: recent_history_inferred
work_segment: valid_pairs, max_display_pairs=2
rounding:
  clock_in: ceil 30m
  clock_out: none
break: none
daily_buckets:
  regular_under_or_equal_8h
  over_8h
leave_count:
  paid=1
  am_paid=0.5
  pm_paid=0.5
report: legacy spreadsheet
```

### `legacy_regular_2026`

院内正社員互換。

```text
period: cutoff day 15
punch_interpretation: recent_history_inferred
work_segment: valid_pairs, max_display_pairs=2
rounding:
  clock_in: ceil 30m
  clock_out: none
break: none
extra_time:
  fixed_time_after: 19:00
  suppress_display_when_note_present: true
leave_count:
  paid=1
  am_paid=0.5
  pm_paid=0.5
report: legacy spreadsheet
```

### `legacy_doctor_2026`

院内ドクター互換。

```text
period: cutoff day 15
punch_interpretation: recent_history_inferred
work_segment: days_only or valid_pairs_for_reference
rounding: not displayed
break: none
count:
  work_days
  paid_leave_days
report:
  show_punch_pairs
  show_missing_punch_flag
  hide_work_minutes
  hide_extra_time
```

### `actual_time_basic`

汎用・法令記録寄り。

```text
period: configurable
punch_interpretation: explicit or inferred
work_segment: valid_pairs
rounding: none
break: punched only
warnings:
  missing_punch
  long_segment
  legal_rest_shortage_candidate
extra_time:
  daily_over_8h_candidate
  weekly_over_40h_candidate
```

## 次の調査候補

- 日本の給与ソフト CSV 連携仕様を調べ、export bucket 名を逆算する。
- 医療/歯科/クリニック特有の勤務形態を調べる。
- 夜勤・宿直・オンコールの扱いを別 ADR 候補として調べる。
- 国内法令上の「丸め」の扱いを厚労省/労基署資料ベースで確認する。
- 有給の半日/時間単位年休の扱いを国内資料で精査する。

## 参照

- TimeTrex: Rounding Policies
  - https://help.timetrex.com/latest/enterprise/Components/Rounding-Policies.htm
- TimeTrex: Break Policies
  - https://help.timetrex.com/latest/enterprise/Components/Break-Policies.htm
- TimeTrex: Overtime Policies
  - https://help.timetrex.com/latest/enterprise/Components/Overtime-Policies.htm
- TimeTrex: Premium Policies
  - https://help.timetrex.com/latest/enterprise/Components/Premium-Policies.htm
- TimeTrex: Policy Groups
  - https://help.timetrex.com/latest/enterprise/Components/Policy-Groups.htm
- TimeTrex: Recalculating TimeSheets
  - https://help.timetrex.com/latest/enterprise/Components/Recalculating-Timesheets.htm
- Frappe HR: Shift Type
  - https://docs.frappe.io/hr/shift-type
- Frappe HR: Attendance Request
  - https://docs.frappe.io/hr/attendance-request
- Odoo 18: Attendances
  - https://www.odoo.com/documentation/18.0/applications/hr/attendances.html
- Kimai: Settings
  - https://www.kimai.org/documentation/configurations.html
