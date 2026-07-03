# 勤怠 policy 仕様インタビュー 01

- 日付: 2026-05-20
- ブランチ: `codex/timesheet-policy-research`
- 目的: policy 仕様を決めるための初回インタビュー結果を記録する。
- スコープ: docs-only。ここに書く「仮決め」は ADR ではなく、次の確認・仕様化の材料。

## 回答

### Q1. システムの主目的

回答:

- 最初は `C. 既存 Excel/Numbers 帳票の置き換え`
- 将来的には `A. 法令・監査に耐える客観的な勤怠記録` と `D. 職場ごとの勤怠運用をできるだけ吸収する基盤`
- 理由: 売り物にするため。

仮決め:

- MVP は院内帳票互換を最優先する。
- ただし DB 設計と policy 境界は、将来の監査性・汎用化を壊さない形にする。
- 既存帳票の丸め・表示ルールは `ReportPolicy` / `PolicyProfile` として扱い、raw 打刻正本とは分離する。

### Q2. raw 打刻の保存

回答:

- raw 打刻は常に実時刻で保存する。
- 丸めは集計・帳票だけに適用する。

仮決め:

- `punch_event.occurred_at` は実打刻時刻。
- 丸め済み時刻を `occurred_at` に保存しない。
- 集計結果には `raw_start/raw_end` と `counted_start/counted_end` のような分離が必要。

### Q3. MVP の給与計算

回答:

- 給与計算そのものはやらない。
- ただし給与ソフト/Excel に渡す分類値は出してよい。

仮決め:

- MVP は給与計算エンジンを持たない。
- `勤務日数`、`有給日数`、`8h 内/8h 超`、`19:00 以降` のような分類・帳票値は扱う。
- 割増率や賃金額は計算しない。

### Q4. 「残業」の扱い

回答:

- 表示だけの問題という認識。
- ただし実務上の意味は事務局が知っているため、現時点では不明。

リスク:

- 「残業」という表示が、法定時間外・所定外・院内独自の 19:00 以降表示のどれを意味するか不明。
- このまま内部名を `overtime` だけにすると、売り物にするときに誤解や仕様破綻を招く。

仮決め:

- 内部分類名は `overtime` 単独にしない。
- 表示ラベルは職場ごとに変更可能にする。
- 院内互換では `19:00 以降` を `legacy_regular_fixed_time_extra` のような内部分類として扱い、帳票表示ラベルだけ `残業` にできるようにする。
- 法定外候補は別分類として後続で扱う。

事務局に確認する質問:

1. 現行 Excel の「残業」は、給与計算上どのように使っていますか。
2. 19:00 以降だけを数える理由は、就業規則・給与規程・単なる帳票便宜のどれですか。
3. 19:00 前に 8 時間を超えて働いた日は、残業扱いになりますか。
4. 19:00 以降でも備考がある日は、なぜ表示上 `-` になりますか。
5. パートの `8h 超` と正社員の `残業` は給与処理上同じ意味ですか、違う意味ですか。

### Q5. 最初に対応すべき職場運用

回答:

- 現在の院内運用を優先する。

仮決め:

- 初期 preset は院内互換を起点にする。
- `legacy_part_time_2026`
- `legacy_regular_2026`
- `legacy_doctor_2026`

### Q6. 休憩

回答:

- 正社員は現行では残業以外を計算していない。
- パートは休憩のときにタイムカードを切っている。

解釈:

- 正社員は勤務時間本体を給与計算に使っていない可能性が高い。
- パートは休憩を打刻 OUT/IN として表現しており、`valid_pairs` 合算で自然に休憩控除される。
- 現行 v1 CSV は最大 2 ペアなので、休憩 1 回までなら対応できるが、複数休憩には弱い。

仮決め:

- MVP のパートは `valid_pairs` で OUT 中を勤務時間から除外する。
- 自動休憩控除は MVP では入れない。
- 正社員は院内互換帳票では残業表示のみを重視し、総勤務時間/休憩控除は参考値に留める。
- 法定休憩不足 warning は将来候補。自動控除とは分ける。

事務局に確認する質問:

1. パートの休憩打刻は、必ず `退勤1 -> 出勤2` として記録されていますか。
2. パートで休憩が 2 回以上ある日はありますか。
3. パートで休憩打刻を忘れた場合、現行ではどう修正していますか。
4. 正社員の休憩は給与・勤怠表上で管理していますか、それとも固定/みなしですか。
5. 正社員の勤務時間合計は今後表示したいですか。

## 初回インタビューから見えた仕様方針

### 固められること

- raw punch は実時刻保存。
- MVP は給与計算しない。
- 既存帳票互換を優先。
- policy は将来販売を考えて拡張可能な境界にする。
- 院内互換 preset を初期対象にする。
- パート休憩は休憩打刻を勤務区間から除外する形で扱う。

### まだ固めないこと

- 「残業」表示の業務意味。
- 正社員の勤務時間・休憩をどこまで扱うか。
- 法定外・所定外・院内独自残業の UI 表示名。
- 自動休憩控除。
- 複数休憩。
- 法定休憩不足 warning。

## 次回インタビュー案

次は事務局向けに、以下だけを確認すればよい。

### 残業

1. 現行 Excel の「残業」は給与処理に使っていますか。
2. 正社員の `19:00 以降` はどの規程に基づいていますか。
3. 8 時間超と 19:00 以降がズレた日はどう扱っていますか。
4. 備考がある日の残業が `-` になる理由は何ですか。

### パート休憩

1. 休憩時は必ずタイムカードを切る運用ですか。
2. 休憩打刻漏れは誰が、どの資料を根拠に修正していますか。
3. 休憩が複数回になる日はありますか。

### 帳票

1. 現行 Excel のどの列を給与・確認・保存に使っていますか。
2. 使っていない列はありますか。
3. 将来売り物にする場合、帳票名や列名は職場ごとに変えたいですか。

## 追加入力: 事務局確認 01

### 回答

1. 現行 Excel の「残業」は給与処理に使っている。
2. 正社員の `19:00 以降` を残業としている根拠は慣例。
3. 正社員が `19:00` 前に 8 時間を超えて働くケースは存在しない。
4. 備考がある日の残業欄が `-` になる理由は未回答。

### 追加解釈

- 院内正社員の `残業` は、法定時間外労働そのものではなく、給与処理に渡す院内慣例の分類値。
- `19:00` 前に 8 時間超となるケースが存在しないなら、院内運用上は `8h 超` と `19:00 以降` のズレを当面考慮しなくてよい。
- ただし売り物化を考えると、内部名を `overtime` に固定するのは危険。`fixed_time_extra_after_19_00` のような policy 出力を、帳票上 `残業` と表示する形がよい。
- 「慣例」由来なので、他職場向けには基準時刻を設定可能にする必要がある。

### 仕様仮決めへの反映候補

`legacy_regular_2026`:

```text
extra_time:
  kind: fixed_time_after
  threshold: "19:00"
  source: workplace_custom
  payroll_export_used: true
  display_label: "残業"
```

### 未解決

- 備考がある日の残業欄が `-` になる理由。
- 備考のどの値が残業表示を抑制するのか。
- 備考で残業表示を抑制しても、給与処理上も残業なし扱いなのか、帳票表示だけなのか。

## 追加入力: 事務局確認 02

### 回答

1. 現行 Excel の「残業」は給与処理に使っている。
2. 正社員の残業計算は、`1年単位の変形労働時間制` の就業規則に関係している。
3. 年間の労働時間が決まっているため、院内運用上 `19:00` 前に 8 時間を超えるケースは存在しない。
4. 備考がある日の残業欄が `-` になる理由は不明。
5. パートの休憩は `退勤 -> 出勤` でタイムカードを切る運用。
6. 現行 Excel の列は `sum` シートで計算に使っている。

### 法令ソース確認

厚生労働省資料では、`1年単位の変形労働時間制` は、1か月を超え1年以内の対象期間を平均して 1 週間あたりの労働時間が 40 時間を超えないことを条件に、業務の繁閑に応じて労働時間を配分する制度と説明されている。
また、労使協定で労働日および労働時間を具体的に特定することが前提とされている。

参照:

- 厚生労働省: 変形労働時間制の概要
  - https://www.mhlw.go.jp/stf/seisakunitsuite/bunya/koyou_roudou/roudoukijun/roudouzikan/henkei.html
- 厚生労働省: 労働基準法第32条の4（1年単位の変形労働時間制）について
  - https://www.mhlw.go.jp/shinsai_jouhou/koyou_roudou/2r9852000001aur4.html
- 厚生労働省・都道府県労働局・労働基準監督署: 1年単位の変形労働時間制パンフレット
  - https://www.mhlw.go.jp/content/001021908.pdf

### 追加解釈

- 事務局確認 01 の「慣例」は、単なる任意運用ではなく、`1年単位の変形労働時間制` に基づく年間所定労働時間・年間カレンダーを日々の帳票へ落とした慣例と解釈し直す。
- 院内正社員の `19:00 以降` は、法定外時間そのものではなく、院内の年間変形労働時間制に基づく給与処理入力値の可能性が高い。
- ただし、システム側が就業規則・労使協定の適法性や年間総労働時間の妥当性を自動判定するのは MVP 範囲外。
- v2 では `annual_variable_working_hours` を将来 policy 候補として明示し、MVP では院内互換として `fixed_time_after_19_00` を出す。
- 売り物化するときは、`1年単位の変形労働時間制` に対応するには年間カレンダー、対象期間、所定労働日、日別所定労働時間、週平均チェック、締め後 lock が必要になる。

### 仕様仮決めへの反映候補

`legacy_regular_2026`:

```text
extra_time:
  kind: fixed_time_after
  threshold: "19:00"
  source: annual_variable_working_hours_local_rule
  payroll_export_used: true
  display_label: "残業"
```

将来の汎用 policy:

```text
annual_variable_working_hours:
  period_start
  period_end
  yearly_scheduled_work_minutes
  work_calendar:
    date
    scheduled_start
    scheduled_end
    scheduled_break_minutes
    scheduled_work_minutes
  validation:
    weekly_average_limit_minutes
    require_labor_management_metadata
```

### 未解決

- `19:00` は就業規則上の所定終業時刻なのか、帳票上の便宜的な残業開始時刻なのか。
- 年間カレンダーまたは労使協定上の日別所定労働時間はどこで管理されているか。
- `sum` シートのどの列が給与処理へ渡るか。
- 備考がある日の残業欄が `-` になる理由。
- 備考欄で `追加残業` を使う実例。

## 追加入力: 事務局確認 03

### 回答

1. `19:00` は就業規則上の所定終業時刻。
2. 年間カレンダーや日別所定労働時間は紙で管理している。
3. `sum` シートをもとに、事務局が紙で給与処理用の計算をしている。

### 追加解釈

- `19:00` は単なる帳票便宜ではなく、正社員の所定終業時刻。
- 現時点の正本は、DB や Excel ではなく、就業規則と紙の年間カレンダー/日別所定労働時間。
- Excel の `sum` シートは給与処理の補助資料であり、最終計算は事務局の紙運用に依存している。
- v2 で売り物化を目指すなら、紙の年間カレンダーを `work_calendar` として構造化する必要がある。

### 仕様仮決めへの反映候補

MVP:

```text
legacy_regular_2026:
  scheduled_end_time: "19:00"
  scheduled_end_source: work_rule_paper_calendar
  fixed_time_extra_after_scheduled_end:
    enabled: true
    display_label: "残業"
    used_for_payroll_workpaper: true
```

将来:

```text
annual_work_calendar:
  source_document: paper_calendar
  version
  effective_period
  days:
    date
    is_work_day
    scheduled_start_time
    scheduled_end_time
    scheduled_work_minutes
    note
```

### 紙運用をデータ化するときの確認項目

1. 年間カレンダーの対象期間はいつからいつまでか。
2. 正社員全員が同じ年間カレンダーか、個人/職種で違うか。
3. 所定開始時刻も固定か。終業 `19:00` だけが重要なのか。
4. 土曜・矯正日・休診日などで所定時間が変わる日があるか。
5. 紙の給与処理で、`sum` シートのどの値をどの欄に転記しているか。
6. 紙計算で補正する例外は何か。
7. 年間カレンダーが途中変更された場合、過去の勤怠を再計算するか。

### 未解決

- 備考がある日の残業欄が `-` になる理由。
- `追加残業` 備考の実際の使い方。
- パートの `sum` シート値が給与計算にどう渡るか。

## 追加入力: 年間カレンダー確認 01

### 回答

1. 年間カレンダーの対象期間は `3月16日` から翌年 `3月15日`。
2. 正社員全員が同じ年間カレンダー。
3. 所定勤務時間は以下。
   - `08:30-12:55`
   - `14:00-16:00`
   - `16:15-19:00`
4. 土曜は就業規則上は出ないことになっているが、振替で平日休み。
5. 紙計算では `sum` シートから `出勤日数`、`有給`、`残業` を転記しているはず。

### 追加解釈

- 年間カレンダーの起算日は勤怠締め期間と同じ `3/16`。
- 正社員は共通の年間カレンダーでよい。
- 1 日の所定勤務は 3 セグメント:
  - 午前: 08:30-12:55 = 4時間25分
  - 午後1: 14:00-16:00 = 2時間
  - 午後2: 16:15-19:00 = 2時間45分
  - 合計: 9時間10分
- 休憩/中断は以下:
  - 12:55-14:00 = 1時間5分
  - 16:00-16:15 = 15分
  - 合計: 1時間20分
- 1 日 9時間10分の所定労働日があるため、年間平均で週40時間以内に収める 1年単位の変形労働時間制の構造と整合する。
- 土曜勤務は通常の所定労働日ではなく、平日休みとの振替として扱う必要がある。

### 仕様仮決めへの反映候補

`legacy_regular_2026`:

```text
annual_work_calendar:
  period:
    starts_on_month_day: "03-16"
    ends_on_month_day: "03-15"
  applies_to:
    employment_type: regular
  default_work_day:
    segments:
      - start: "08:30"
        end: "12:55"
      - start: "14:00"
        end: "16:00"
      - start: "16:15"
        end: "19:00"
    scheduled_work_minutes: 550
    scheduled_break_minutes: 80
  scheduled_end_time: "19:00"
  payroll_workpaper_fields:
    - work_days
    - paid_leave_days
    - fixed_time_extra_after_19_00
```

振替:

```text
calendar_day_kind:
  regular_work_day
  regular_non_work_day
  transferred_work_day
  transferred_day_off
```

### 未解決

- 土曜勤務と平日休みの振替は、紙カレンダー上で事前に決まっているのか、後から実績で決めているのか。
- 土曜勤務日の所定時間は平日と同じ `08:30-19:00` か、短縮か。
- 平日休みは `有給` ではなく `振替` として扱うのか。
- `AM振替` / `PM振替` の使い方。
- 正社員の出勤日数は、振替休日を除き、土曜振替勤務を含めるか。

## 追加入力: 振替確認 01

### 回答

1. 年間カレンダー上は土曜勤務なし。土曜勤務は後から実績で振替している。
2. 土曜勤務は事情により `09:00-12:00`。
3. 振替で休む平日は `振替` 扱い。
4. `PM振替` を実際に使う。
5. 出勤日数は土曜勤務により増えるが、1年変形制なので最終的には時間で管理している。

### 追加解釈

- 年間カレンダーは予定上の正本であり、土曜は原則非勤務日。
- 土曜勤務は `actual adjustment` または `transfer work` として後から発生する。
- 土曜勤務の所定扱いは平日と同じ 9時間10分ではなく、実績上 `09:00-12:00` の 3時間。
- 振替休日は、実際には土曜勤務 3時間より長く休む場合がある。
- そのため、振替は「土曜 3時間勤務と平日 3時間休みを厳密相殺」ではなく、年間変形制の総時間管理内で事務局が調整している可能性が高い。
- 出勤日数は増えるが、給与/労務上の主管理軸は年間労働時間。

### 仕様仮決めへの反映候補

予定カレンダー:

```text
calendar_day:
  date
  planned_day_kind:
    work_day
    non_work_day
  planned_segments
  planned_work_minutes
```

実績調整:

```text
work_adjustment:
  adjustment_type:
    transfer_work
    transfer_day_off
    pm_transfer_day_off
  date
  minutes
  reason
  source:
    admin
    lineworks
    paper_import
```

院内互換:

```text
saturday_transfer_work:
  default_segments:
    - start: "09:00"
      end: "12:00"
  default_work_minutes: 180
  source: actual_adjustment

pm_transfer:
  attendance_note: "PM振替"
  source: manual_note
```

### v2 設計への示唆

- `shift_assignment` だけでは不足する。予定カレンダーと実績調整を分けたい。
- `振替` は leave でも absence でもなく、annual variable working hours の調整イベント。
- `PM振替` は半日有給とは別の attendance note / adjustment type。
- 月次表には `出勤日数` と `労働時間` の両方が必要。1年変形制の正本判断は時間側。
- 土曜勤務を単に `holiday_work` にすると、振替との対応が失われる。

### 未解決

- 土曜勤務と PM振替は 1 対 1 で紐づけて管理しているか。
- 土曜勤務の `09:00-12:00` を超えた場合、超過分は残業/追加残業/別調整になるか。
- PM振替で休む時間帯は固定か。
- `振替` と `PM振替` のどちらをいつ使うか。
- 年間総時間の紙計算では、土曜勤務 180 分と PM振替を何分として扱うか。

## 追加入力: 振替確認 02

### 回答

1. 土曜勤務と振替休みは 1 対 1 で紐づけて管理している。AM/PM を選べる。
2. `PM振替` で休む時間帯は固定。
3. 紙の年間時間計算では、`PM振替` は 5 時間として数える。
4. 土曜勤務は `13:00` を過ぎたら残業。根拠は就業規則。
5. 使い分けは `1日振替` または `PM振替`。

### 追加解釈

- 振替は 1 対 1 で対応づける必要がある。
- 振替休みは `full_day_transfer_day_off` と `pm_transfer_day_off` の少なくとも 2 種類。
- AM/PM を選べるとのことなので、`am_transfer_day_off` もデータモデル上は許容した方がよい。ただし実運用では `PM振替` が主。
- `PM振替` は 5 時間休みとして計算する。
- 土曜勤務は通常 `09:00-12:00` の 3 時間だが、`13:00` を超えると就業規則上の残業が発生する。
- 土曜の `12:00-13:00` の扱いは未確認。休憩/猶予/勤務扱いのどれかを確認する必要がある。

### 仕様仮決めへの反映候補

```text
transfer_pair:
  id
  transfer_work_date
  transfer_day_off_date
  transfer_day_off_kind:
    full_day
    am
    pm
  source
  reason
```

```text
saturday_transfer_work_policy:
  planned_segments:
    - start: "09:00"
      end: "12:00"
  counted_work_minutes: 180
  overtime_after: "13:00"
  overtime_source: work_rule
```

```text
transfer_day_off_policy:
  full_day:
    counted_minutes: 550 # 平日所定 9時間10分
  pm:
    counted_minutes: 300 # 5時間
  am:
    counted_minutes: null # 要確認。選択肢としては存在する
```

### v2 設計への示唆

- `attendance_note` だけに `PM振替` と文字列保存するのでは、対応する土曜勤務との 1 対 1 関係を表現できない。
- 最低でも将来は `transfer_pair` か `work_adjustment_link` が必要。
- MVP では既存帳票互換として備考 `PM振替` を残し、手動管理を継続してもよい。
- 売り物にするには、振替勤務と振替休みの対応づけ UI が必要。
- `PM振替 = 5時間` は就業規則/院内運用由来の policy 値。

### 未解決

- `AM振替` を使う場合の時間換算。
- `PM振替` の固定時間帯。推定では `14:00-19:00` だが、途中 `16:00-16:15` の扱いを含め確認が必要。
- 土曜 `12:00-13:00` の扱い。
- 土曜 `13:00` 超過時の残業は、`13:00` 以降全てか、実退勤から `13:00` までか。
- 1日振替の時間換算は 9時間10分でよいか。

## 追加入力: 振替時間確認 01

### 回答

1. `PM振替` の固定時間帯は `14:00-19:00`。
2. `AM振替` は `08:30-13:00` として 4時間30分扱い。`12:55` までとすると 5分が失われるため。
3. `1日振替` は `08:30-12:55`、`14:00-16:00`、`16:15-19:00`。
4. 土曜 `12:00-13:00` は休憩/空白時間。`13:00` を超えた分だけ残業。
5. 土曜残業は `13:00` 以降の時間を残業として数える。

### 追加解釈

- `AM振替` と `PM振替` は、どちらも 4時間30分/5時間のような丸い管理値を持つ。
- `AM振替` は実際の午前セグメント `08:30-12:55` ではなく、計算上 `08:30-13:00` として 270 分。
- `PM振替` は `14:00-19:00` として 300 分。途中の `16:00-16:15` は PM振替の計算では控除しない。
- `1日振替` は通常勤務日の所定 3 セグメントそのもの。合計 550 分。
- 土曜勤務は `09:00-12:00` が通常範囲、`12:00-13:00` は残業にならず、`13:00` 以降が残業。

### 仕様仮決めへの反映候補

```text
transfer_day_off_policy:
  full_day:
    segments:
      - start: "08:30"
        end: "12:55"
      - start: "14:00"
        end: "16:00"
      - start: "16:15"
        end: "19:00"
    counted_minutes: 550
  am:
    display_time_range: "08:30-13:00"
    counted_minutes: 270
  pm:
    display_time_range: "14:00-19:00"
    counted_minutes: 300
```

```text
saturday_transfer_work_policy:
  regular_range:
    start: "09:00"
    end: "12:00"
    counted_minutes: 180
  non_counted_gap:
    start: "12:00"
    end: "13:00"
  extra_time_after:
    threshold: "13:00"
    display_label: "残業"
    source: work_rule
```

### v2 設計への示唆

- 「時間帯」と「計算分数」は必ずしも一致しない。特に `AM振替` と `PM振替`。
- `TransferPolicy` は `segments` と `counted_minutes` を別々に持てる形がよい。
- 土曜の残業開始は通常勤務終了 `12:00` ではなく `13:00`。ここにも就業規則由来の grace/window がある。
- 同じ `残業` 表示でも、平日は `19:00` 以降、土曜は `13:00` 以降。日種別ごとの fixed_time_extra policy が必要。

### 未解決

- 土曜勤務が `09:00` より前に始まった場合の扱い。
- 土曜勤務が `12:00-13:00` の間で終わった場合、出勤日数や時間は 3時間固定か、実時間か。
- `PM振替` を取った日の出勤日数は 1 日出勤扱いか、0.5 日か、別集計か。
- 振替 pair の未消化/過消化を月次または年間でどう表示するか。

## 追加入力: 振替ニュアンス確認 01

### 回答

1. `PM振替` を取った平日の出勤日数については、前提として「土曜出勤が、ないこと、になっている」というニュアンス。
2. 土曜勤務が `12:00-13:00` の間で終わった場合、勤務時間は実時間。
3. 土曜勤務が `09:00` より前に始まった場合は丸めている。
4. 土曜勤務後に振替休みを取らない未消化状態はない。必ず消化する。
5. 先に振替休みを取り、後で土曜勤務することはある。

### 追加解釈

- 「土曜出勤が、ないこと、になっている」とは、制度・帳票・給与処理上、土曜勤務を通常の出勤日数として表に出すのではなく、振替休みとセットにして年間時間内で調整するという意味。
- したがって `transfer_work` は内部的には必要だが、通常の `work_day_count` に単純加算すると運用ニュアンスとズレる可能性がある。
- 土曜勤務と振替休みは 1 対 1 だが、順序は `work -> day_off` だけでなく `day_off -> work` もある。
- 未消化は運用上発生しない前提。ただしシステムでは未消化/未対応を warning できるとよい。
- 土曜の `12:00-13:00` 終了は実時間。つまり土曜通常範囲は 3時間固定ではなく、実打刻に基づく。ただし `13:00` 以降だけが残業。
- 土曜 `09:00` 前の早出には丸め policy が存在する。丸め単位/方向は未確認。

### 仕様仮決めへの反映候補

`transfer_pair`:

```text
transfer_pair:
  id
  status:
    planned
    matched
    unmatched_warning
  order:
    work_then_day_off
    day_off_then_work
  transfer_work_date
  transfer_day_off_date
  transfer_day_off_kind:
    full_day
    am
    pm
  visible_as_regular_work_day: false
```

土曜勤務:

```text
saturday_transfer_work_policy:
  counted_minutes:
    mode: actual_until_extra_threshold
  extra_time_after:
    threshold: "13:00"
  early_start_rounding:
    enabled: true
    details: unresolved
```

月次/年間表示:

```text
work_day_count_policy:
  transfer_work:
    add_to_regular_work_days: false # 「土曜出勤がないこと」のニュアンスを守る候補
  transfer_day_off:
    display_as_adjustment: true
```

### v2 設計への示唆

- `出勤日数` は単純な打刻あり日数ではない。特に振替勤務は別扱い。
- `work_minutes` と `work_day_count` は別 policy。
- 振替は順序非依存で pair 管理する必要がある。
- 「土曜出勤を表に出さない」帳票 policy が必要。
- ただし raw punch と internal adjustment には土曜勤務を残す。監査性のため、実際に起きた勤務を消さない。

### 未解決

- 土曜 `09:00` 前の丸め単位と方向。
- `transfer_work` を `sum` シートの出勤日数に含めているか。
- `PM振替` 取得日の `sum` シート上の出勤日数の扱い。
- 土曜勤務と振替休みの pair を紙ではどのように確認しているか。

## 追加入力: 出勤日数と紙シフト確認 01

### 回答

1. 土曜 `09:00` 前に始まった場合は `09:00` 扱い。
2. 正社員は出勤日数が関係ないため、`sum` シート上の土曜勤務日の扱いは気にしていなかった。
3. 正社員は出勤日数が関係ないため、`PM振替` 取得日の出勤日数扱いも気にしていなかった。
4. 土曜勤務と振替休みの 1 対 1 対応は紙のシフト表で管理している。

### 追加解釈

- 院内正社員では、`出勤日数` は給与処理上の主要値ではない。
- 正社員の主管理軸は、1年単位の変形労働時間制に基づく時間管理と、紙シフト表上の振替対応。
- `sum` シートの `出勤日数` は存在するが、正社員については参考値または未使用に近い。
- 土曜早出は `09:00` へ丸める。つまり `09:00` より前の勤務は残業/勤務時間として数えない運用。
- 紙のシフト表が、年間カレンダーに対する実績調整の正本になっている。

### 仕様仮決めへの反映候補

```text
legacy_regular_2026:
  primary_payroll_fields:
    - paid_leave_days
    - fixed_time_extra_after_scheduled_end
    - transfer_adjustments
  secondary_fields:
    - work_day_count
```

```text
saturday_transfer_work_policy:
  early_start_rounding:
    before: "09:00"
    counted_as: "09:00"
```

```text
transfer_pair_source:
  current_source: paper_shift_sheet
  future_system_model: transfer_pair
```

### v2 設計への示唆

- `出勤日数` は雇用区分/policy profile により「主要集計値」か「参考値」かが変わる。
- 正社員の `MonthlyTimesheet` には `work_day_count` より `scheduled_minutes`、`actual_counted_minutes`、`transfer_adjustment_minutes`、`fixed_time_extra_minutes` が必要。
- 紙シフト表を将来取り込むなら、`shift_assignment` だけでなく `transfer_pair` の入力 UI が必要。

### 未解決

- 正社員の紙計算で、有給日数は給与処理にどう影響するか。
- 正社員の年間時間管理で、`PM振替` 300 分と土曜実勤務時間の差分をどう扱うか。
- 紙シフト表を v2 に移す優先度。

## 追加入力: 正社員有給と紙併用確認 01

### 回答

1. 正社員の有給を何時間として扱うかは気にしていなかった。むしろ年間時間調整の余白として使っている。
2. 正社員でも `AM有給` / `PM有給` を使う。
3. `PM振替` 300 分に対して土曜勤務 180 分だった場合の差分は、年間時間の中で吸収している。
4. 紙シフト表は MVP では併用する方向。

### 追加解釈

- 正社員の有給・半日有給は、厳密な時間換算よりも日数/半日数として管理されている。
- 年間時間管理には、紙運用上の調整余白がある。
- `PM振替` と土曜実勤務時間の差分はシステムが厳密にエラー扱いしない方が現行運用に合う。
- MVP で紙シフト表を完全置換するのはスコープ過大。

### 仕様仮決めへの反映候補

```text
legacy_regular_2026:
  leave_count:
    paid: 1.0 day
    am_paid: 0.5 day
    pm_paid: 0.5 day
    counted_minutes: not_calculated_in_mvp
  transfer_balance:
    strict_minute_reconciliation: false
    source_of_truth: paper_shift_sheet
  annual_variable_working_hours:
    full_engine: out_of_mvp
    mvp_mode: paper_assisted_summary
```

### v2 設計への示唆

- `LeavePolicy` は `days` と `minutes` を分ける必要がある。
- MVP の正社員有給は `paid_leave_days` の集計まで。
- 年間変形制の完全な時間整合チェックは MVP 外。
- 紙シフト表併用のため、v2 は「紙計算に必要な補助集計を出す」位置づけで始める。

### 未解決

- 正社員の `AM有給` / `PM有給` の固定時間帯。
- 紙計算へ渡す最終帳票フォーマット。
- MVP で振替 pair の入力だけは持つか、完全に紙任せにするか。

## 追加入力: 正社員有給時間帯と MVP 振替スコープ確認 01

### 回答

1. 正社員の `AM有給` は `08:30-13:00` 扱い。
2. 正社員の `PM有給` は `14:00-19:00` 扱い。
3. MVP では振替 pair の入力画面は作らず、備考欄の `PM振替` 表示だけでよい。

### 追加解釈

- 正社員の半日有給は、半日振替と同じ時間帯で扱える。
- `AM有給` は 270 分相当、`PM有給` は 300 分相当。ただし MVP では時間換算より日数 0.5 集計が主。
- MVP の振替は `attendance_note` として表示・集計するに留め、土曜勤務との pair 管理は紙シフト表に残す。

### 仕様仮決めへの反映候補

```text
legacy_regular_2026:
  leave_count:
    paid: 1.0 day
    am_paid: 0.5 day
    pm_paid: 0.5 day
  half_day_time_ranges:
    am:
      display_time_range: "08:30-13:00"
      reference_minutes: 270
    pm:
      display_time_range: "14:00-19:00"
      reference_minutes: 300
  transfer_pair_input:
    mvp: false
    source_of_truth: paper_shift_sheet
  attendance_notes:
    include:
      - PM振替
```

### v2 設計への示唆

- MVP では `transfer_pair` テーブルを作らず、将来拡張候補として ADR に残すのが妥当。
- `attendance_note` は文字列ではなく enum 化候補。少なくとも `paid`, `am_paid`, `pm_paid`, `pm_transfer` は構造化したい。
- 半日有給の時間帯は日数集計とは別に、将来の年間時間チェック用 reference として残せる。

### 未解決

- 紙計算へ渡す最終帳票フォーマット。
- パートの休憩・有給・残業集計の詳細。
