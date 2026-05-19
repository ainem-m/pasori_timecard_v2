# QUESTIONS

実装を始める前に、ドキュメント全体を読んで見つかった未整合・未確定事項を整理した。
ここで回答をもらい、その内容に従って必要なら ADR / spec を更新してから実装に入る。

## 1. Terminal の API token 保存方針

### 論点

`docs/spec/07_security.md` では Terminal の API token を
`~/.config/pasori-timecard-terminal/config.toml` に平文保存すると書かれている。
一方で `AGENTS.md` と同ファイル後半では、設定ファイル / コードに平文 secret を
置くことを禁止している。

### 衝突している記述

- `AGENTS.md`: 設定ファイル / コードに平文シークレットを置かない
- `docs/spec/07_security.md`: Terminal token を `config.toml` に平文保存
- `docs/spec/07_security.md`: 平文 secret のコード / 設定ファイル混入は禁止

### 回答してほしいこと

次のどれを正式方針にするか決めてほしい。

[ ] 1. Terminal token は例外として `config.toml` に平文保存してよい
[ ] 2. Terminal token は OS の秘密情報ストアに保存する
[x] 3. Terminal token は暗号化して設定ファイルに保存する
[ ] 4. 別案

## 2. `card_id` の意味を統一したい

### 論点

サーバー正本の `punch_event.card_id` は `card(id)` を参照しているので、UUID に見える。
しかしドメインでは `CardId` を FeliCa IDm の hex 表現として扱っている。
さらに Terminal の local cache には `card_identifier` しかなく、`pending_punch` では
`card_id` を必須にしている。

### 衝突している記述

- `AGENTS.md`: `CardId(pub String)` は FeliCa IDm の hex 表現
- `docs/spec/06_data_model.md`: `punch_event.card_id` は `REFERENCES card(id)`
- `docs/spec/06_data_model.md`: `pending_punch.card_id` は必須
- `docs/spec/06_data_model.md`: `card_cache` にあるのは `card_identifier`

### 回答してほしいこと

次のどれを正式方針にするか決めてほしい。

[x] 1. `card_id` は常に card table の UUID を指す。FeliCa IDm は `card_identifier` と呼ぶ
[ ] 2. `card_id` は常に FeliCa IDm を指す。DB 参照列名を見直す
[ ] 3. サーバー層では UUID、ドメイン層では FeliCa IDm を使うが、名前を分けて明示する
[ ] 4. 別案

## 3. オフライン打刻時の確認 UI と必要キャッシュ

### 論点

仕様では確認 UI に推定種別と直近 5 件の打刻履歴を出すことになっている。
しかしオフライン時の local cache には、その判定に必要な recent punches がない。
従業員名だけ local cache から出せる記述になっている。

### 衝突している記述

- `docs/spec/01_nfc_and_punch.md`: 確認 UI に推定種別と直近 5 件履歴を表示
- `docs/spec/01_nfc_and_punch.md`: オフライン時は従業員名を local cache から表示
- `docs/spec/06_data_model.md`: Terminal cache に recent punch cache がない

### 回答してほしいこと

オフライン時の仕様を次のどれにするか決めてほしい。

[x] 1. online と同じ UI を維持する。そのため recent punches を Terminal にキャッシュする
[ ] 2. offline 時は簡略 UI にする。推定種別や履歴は出さない、または制限付きで出す
[ ] 3. offline 時は最後の既知履歴だけを使って簡易推定する
[ ] 4. 別案

## 4. 監査ログは設定で OFF にできるか

### 論点

`docs/spec/05_audit_and_backup.md` では audit 対象イベントは「設定で ON/OFF 可能」と
書かれているが、`AGENTS.md` の DoD / NO-GO では監査対象イベントの append が必須。
このままだと、設定で OFF にした瞬間に確定事項に反する。

### 衝突している記述

- `docs/spec/05_audit_and_backup.md`: 監査ログは設定で ON/OFF 可能
- `AGENTS.md`: 監査対象イベントを `audit_log` に残さないのは禁止
- `AGENTS.md`: 監査対象操作を追加した場合は `audit_log` append が必須

### 回答してほしいこと

次のどれを正式方針にするか決めてほしい。

[ ] 1. 監査対象イベントは常時 ON。設定で無効化できない
[ ] 2. 必須監査イベントだけ常時 ON。補助イベントだけ設定で ON/OFF 可
[x] 3. すべて設定で ON/OFF 可とし、AGENTS 側の定義を改める
[ ] 4. 別案

## 5. `ShiftChangeLog` を独立テーブルで持つか

### 論点

`docs/spec/03_shift.md` には `ShiftChangeLog` があり、audit_log とは別の専用ログを
持つと書かれている。一方、全体方針は `audit_log` を append-only の監査基盤として
使う前提で、最低限必要なテーブル一覧にも repository trait にも `ShiftChangeLog`
は存在しない。

### 衝突している記述

- `docs/spec/03_shift.md`: `ShiftChangeLog` を独立して持つ
- `AGENTS.md`: 最低限必要なテーブルに `shift_change_log` はない
- `AGENTS.md`: 監査ログは `audit_log` append-only
- `AGENTS.md`: Repository traits に `ShiftChangeLog` 用の口がない

### 回答してほしいこと

次のどれを正式方針にするか決めてほしい。

[ ] 1. シフト変更も `audit_log` に統一する。`ShiftChangeLog` は作らない
[ ] 2. `ShiftChangeLog` を別テーブルとして正式採用する。ADR と trait を追加する
[x] 3. MVP は `audit_log` のみ、post-MVP で必要なら `ShiftChangeLog` を追加する
[ ] 4. 別案

## 6. `ReaderBackend` の配置先を統一したい

### 論点

`ReaderBackend` の配置が `core/src/reader.rs` と `crates/core/src/port/reader.rs` で
食い違っている。Repository や Notifier も含め、`core` の公開面をどこに置くかを
揃えたい。

### 衝突している記述

- `AGENTS.md`: `ReaderBackend` は `core/src/reader.rs`
- `docs/adr/0006-project-structure.md`: `crates/core/src/port/reader.rs`

### 回答してほしいこと

次のどれを正式方針にするか決めてほしい。

[ ] 1. `core/src/reader.rs`, `repo.rs`, `notify.rs` のように直下に置く
[ ] 2. `crates/core/src/port/` 配下にまとめる
[x] 3. 別案 -> 判断不能。どうしたらいい？

## 7. Cloudflare Tunnel は独自ドメイン必須か

### 論点

Cloudflare Tunnel の運用要件がファイル間で揺れている。
`docs/spec/05_audit_and_backup.md` では `trycloudflare.com` も候補に見えるが、
`docs/spec/07_security.md` では独自ドメイン必須と書かれている。

### 衝突している記述

- `docs/spec/05_audit_and_backup.md`: `*.trycloudflare.com` も可、本番は独自ドメイン推奨
- `docs/spec/07_security.md`: DNS ルーティングに独自ドメイン必須

### 回答してほしいこと

次のどれを正式方針にするか決めてほしい。

[ ] 1. 本番も開発も独自ドメイン必須
[x] 2. 本番は独自ドメイン必須、開発/検証だけ `trycloudflare.com` 可
[ ] 3. `trycloudflare.com` でも運用可
[ ] 4. 別案

## 実装前に最低限確定したい項目

優先度が高いのは次の 4 つ。

1. Terminal token 保存方針
2. `card_id` / `card_identifier` の意味
3. オフライン時の確認 UI とキャッシュ要件
4. 監査ログの ON/OFF 可否

この 4 つが固まると、最初のマイグレーション、core の型、Terminal の local cache 設計に入れる。
