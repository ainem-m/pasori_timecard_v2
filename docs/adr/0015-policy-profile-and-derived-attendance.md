# ADR 0015: 勤怠 policy profile と raw/derived 分離を採用する

- **日付**: 2026-06-03
- **状態**: Accepted
- **関連**: docs/spec/02_attendance.md, docs/research/2026-05-20-attendance-policy-deep-dive.md, docs/research/2026-05-20-policy-interview-01.md

## 背景

既存 v1 帳票と事務局インタビューから、同じ `残業`、`有給`、`振替`、`休憩` という語でも、雇用区分や帳票によって意味が異なることが分かった。

- 正社員は 1 年単位の変形労働時間制を前提に、紙の年間カレンダーと紙シフト表を併用している
- パートは出勤時刻を 30 分切り上げ、退勤時刻は丸めず、休憩時に退勤/出勤を切る
- ドクターは勤務時間より出勤日数・有給日数が主集計で、勤務時間は参考表示で足りる
- v2 は将来売り物にするため、職場ごとの運用を吸収できる境界が必要

一方で、勤怠正本である `punch_event.occurred_at` を帳票用の丸め時刻で上書きすると、客観的な打刻記録と監査性が失われる。

## 決定

`employment_type` に直接計算ロジックを埋め込まず、従業員には **policy profile** を適用する。
MVP では policy profile をコード/仕様上の preset として扱い、DB 設定 UI や汎用 policy editor は導入しない。

raw と derived を以下のように分離する。

- raw:
  - `punch_event.occurred_at`
  - 打刻 source (`nfc`, `manual`, `import`, `local_cached` など)
  - correction/audit の根拠
- derived:
  - 丸め後の集計用時刻
  - 勤務区間
  - 休憩控除結果
  - 有給日数
  - 帳票上の `残業` などの補助分類値
  - CSV / Excel export 用の表示値

`punch_event.occurred_at` は常に実時刻を保存し、丸め済み時刻で上書きしない。

## 用語

### PolicyProfile

従業員または雇用区分に割り当てる policy preset。
MVP では以下の preset を持つ。

- `legacy_regular_2026`
- `legacy_part_time_2026`
- `legacy_doctor_2026`

### Derived Attendance

raw punch と policy profile から算出される月次・日次の表示/集計値。
derived attendance は DB 正本ではなく、再計算可能なビューとして扱う。

### 表示ラベルと内部分類

帳票上の `残業` は表示ラベルである。
内部では `fixed_time_extra`、`over_8h`、`scheduled_extra`、`statutory_overtime_candidate` などの分類に分ける。
MVP で給与計算エンジンは作らない。

## 非目的

MVP では以下を行わない。

- policy profile の汎用編集 UI
- 任意スクリプト/プラグインによる勤怠計算
- 給与計算エンジン
- 年間変形労働時間制の法令適合判定
- 年間総労働時間の完全自動整合
- 振替勤務と振替休みの 1 対 1 管理 UI
- 丸め済み時刻の `punch_event` への保存

## 結果

- raw 打刻の監査性を保ったまま、院内帳票互換の丸めや補助集計を実装できる
- `employment_type` と勤怠計算ロジックの結合を避けられる
- 将来、職場ごとの policy preset を追加しやすい
- UI 設定だけで全勤怠ルールを表現しようとする複雑化を避けられる

## 代替案と却下理由

- **雇用区分ごとに if 文で集計する**:
  MVP は速いが、売り物化時に職場ごとの例外を吸収しにくい。
- **設定画面の条件ビルダーで全て表現する**:
  勤怠ルールの優先順位、監査、紙併用、表示ラベルが複雑になりすぎる。
- **任意スクリプト/プラグインを MVP で導入する**:
  raw 打刻や監査ログの整合性を壊すリスクが高い。
