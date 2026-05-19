# 手動 E2E / 実機検証チェックリスト

## 目的

この文書は、`pasori_timecard_v2` の手動または半手動で行う実機検証の
**実施手順**を定義する。

検証要求と合格基準の正本は `docs/spec/` であり、この文書はそれを実施可能にする
従属文書として扱う。

## 対応する要求

- `docs/spec/01_nfc_and_punch.md`
  - オンライン打刻フロー
  - オフライン打刻フロー
  - 未登録カードの取扱
  - NTP 同期チェック
- `docs/spec/07_security.md`
  - Terminal token
  - Bitwarden 運用
  - API token を Authorization header で送ること
- `docs/spec/04_lineworks.md`
  - LINE WORKS callback
  - 照会・修正申請
  - 自動承認 / 要承認フロー
- `docs/spec/05_audit_and_backup.md`
  - 監査ログ確認
  - NTP 運用
- `docs/adr/0013-verification-doc-location.md`
  - `docs/spec/` を検証要求の正本とする
  - `docs/verification/` を手順・証跡の置き場とする

## 自動 E2E と実機 E2E の違い

| 種別 | 目的 | 使用するもの | この文書での扱い |
|---|---|---|---|
| 自動 E2E | UI と API の回帰確認を機械的に繰り返す | WebDriverIO / tauri-driver / mock reader / test DB | 実行対象外。結果があれば証跡として参照してよい |
| 実機 E2E | PaSoRi、実カード、実 Server、実 Terminal の結合を確認する | PaSoRi RC-S380、登録済みカード、未登録カード、Bitwarden 由来の token | 本文の主対象 |
| 半手動 offline E2E | Server 停止、再接続、pending sync の復旧を人が観測する | 実 Terminal、実 Server、local cache、Admin 画面 | シナリオ 4 で扱う |

mock reader や自動注入イベントだけで通った確認は、この文書では「実機 E2E」と呼ばない。

macOS ローカルでは公式 `tauri-driver` が desktop WebDriver をサポートしない。
そのため `pnpm -C web/terminal test:e2e` は Playwright + Vite + mocked Tauri command/event の
自動 UI 回帰確認として扱う。Tauri 実ウィンドウの WebDriverIO 確認は
`pnpm -C web/terminal test:e2e:tauri` を Linux / Windows の対応環境で実行する。

## 実施環境

### 共通前提

| 項目 | 必須条件 |
|---|---|
| 作業場所 | Server と Terminal の画面、ログ、Admin 画面を同時に確認できること |
| Server | ローカルまたは検証機で起動できること |
| Terminal | Tauri terminal を起動できること |
| Admin | Admin アカウントでログインできること |
| 時刻 | Terminal と Server が NTP 同期済みで、時刻差が ±10 秒以内であること |
| 対象 commit | 検証対象の commit hash を記録できること |

### 実機 PaSoRi 前提

| 項目 | 必須条件 |
|---|---|
| Reader | PaSoRi RC-S380 を OS が PC/SC reader として認識していること |
| カード A | 事前に従業員へ紐付け済みで、オンライン打刻に使用できること |
| カード B | 未登録カードとして扱えること |
| 置き方 | カードを reader 中央に 1 枚だけ置けること |
| 連続スキャン | 同一カードを 5 秒以内に再スキャンする確認ができること |

### Bitwarden item 名

実シークレット値は記録しない。証跡には item 名と読み込み成否だけを残す。

| 用途 | 既定 item 名または環境変数名 | 使用箇所 |
|---|---|---|
| Terminal API token | `terminal-api-token` (`BW_TERMINAL_TOKEN_ITEM` 未指定時) | `scripts/bw-run-terminal.sh` |
| LINE WORKS Bot Secret | `lineworks-bot-secret` | `scripts/bw-run-server.sh` |
| LINE WORKS API Token | `lineworks-api-token` | `scripts/bw-run-server.sh` |
| LINE WORKS Bot ID | `lineworks-bot-id` | `scripts/bw-run-server.sh` |
| Database encryption key | `db-encryption-key` | 将来用。現時点では値を記録しない |

### LINE WORKS 前提

- LINE WORKS 検証用ユーザーを 2 名用意する
- 1 名は従業員に紐付け済みにする
- 1 名は未紐付けのままにする

## 起動方法

Server:

```bash
scripts/bw-run-server.sh
```

Terminal:

```bash
scripts/bw-run-terminal.sh
```

補足:

- `scripts/bw-run-server.sh` は Bitwarden から `LINEWORKS_*` を注入する
- `scripts/bw-run-terminal.sh` は Bitwarden から `TERMINAL_API_TOKEN` を注入する
- `SERVER_API_URL` 未指定時は `http://localhost:8080/api`
- `BW_TERMINAL_TOKEN_ITEM` 未指定時は `terminal-api-token`

## 証跡

各シナリオで最低限以下を残す。

- 実施日時
- 実施者
- 対象 commit hash
- Pass / Fail / Blocked
- server log の該当部分
- terminal log の該当部分
- 必要な画面スクリーンショット
- 必要なら DB 確認結果
- 使用した Bitwarden item 名

実シークレット値、API token、cookie、LINE WORKS token、署名値は証跡に残さない。

## 判定区分

| 区分 | 意味 |
|---|---|
| Pass | 仕様の期待結果を満たし、必要な証跡が残っている |
| Fail | 操作は完了したが、期待結果を満たさない |
| Blocked | 機材、認証情報、外部サービス、環境不備で操作を完了できない |

Fail または Blocked の場合は、どの手順で止まったか、期待結果、実際の結果、参照した log を記録する。

## シナリオ 1: Admin ログイン

### 操作

1. Server を起動する
2. Admin 画面を開く
3. 未ログイン状態を確認する
4. 正しい資格情報でログインする
5. 誤った資格情報でも 1 回試す

### 確認方法

- ブラウザ画面を確認する
- 必要なら server log を確認する

### 合格判定

- 未ログイン時にログインフォームが表示される
- 正しい資格情報でログイン後、dashboard が表示される
- 誤った資格情報ではログイン成功しない

### 証跡

- ログイン前画面
- ログイン後画面
- 誤ログイン時の画面

## シナリオ 2: PaSoRi RC-S380 登録済みカードの実機打刻 happy path

Traceability:

- `docs/spec/01_nfc_and_punch.md` の「オンライン時」
- `docs/spec/01_nfc_and_punch.md` の「確認 UI の仕様」
- `docs/spec/01_nfc_and_punch.md` の「連続スキャン無視」
- `docs/spec/07_security.md` の「Terminal → Server (API)」

### 環境前提

- PaSoRi RC-S380 が接続済みで、OS が PC/SC reader として認識している
- カード A が登録済み従業員に紐付いている
- Terminal は `scripts/bw-run-terminal.sh` で起動している
- `BW_TERMINAL_TOKEN_ITEM` は未指定なら `terminal-api-token` を使う
- Server は Terminal token を受け付ける状態で起動している
- Admin 画面で打刻一覧または従業員別の直近打刻を確認できる

### 操作

1. Server を起動する
2. Terminal を起動する
3. Admin 画面にログインする
4. Terminal が待受画面になっていることを確認する
5. カード A を PaSoRi RC-S380 の中央に 1 枚だけ置く
6. Terminal の確認 UI を確認する
7. 30 秒待つ、または OK 長押しで確定する
8. Terminal が打刻完了表示になり、待受画面へ戻ることを確認する
9. Admin 画面の打刻一覧を確認する
10. 5 秒以内に同じカード A を再度置き、連続スキャンが無視されることを確認する
11. 5 秒以上待って同じカード A を再度置き、推定種別の反転を確認する
12. 2 回目の確認 UI を確定し、Admin 画面で 2 件目の打刻を確認する

### 確認方法

- Terminal 画面で従業員名、時刻、推定種別を見る
- Admin 画面で打刻一覧を見る
- terminal log で RC-S380 からの card scan と 5 秒以内の抑止を確認する
- server log で `POST /api/punches` が token 認証を通過していることを確認する
- 5 秒以内の再スキャンでは Server へ送信されていないことを log で確認する

### 合格判定

- Terminal に登録済み従業員として表示される
- 確認 UI に推定種別が表示される
- 確定後に打刻完了表示になる
- Admin 画面に打刻が 1 件追加される
- 5 秒以内の同一カード再スキャンでは新しい打刻が追加されない
- 次回スキャン時に推定種別が期待どおり反転する
- 2 回目の確定後、Admin 画面に 2 件目の打刻が追加される
- API token の実値が log や画面に出力されない

### 証跡

- PaSoRi RC-S380 を使っていることが分かる作業写真またはメモ
- Terminal の確認 UI
- Terminal の打刻完了画面
- Admin の打刻一覧画面
- terminal log の card scan、suppression、punch submit 該当部分
- server log の punch 作成該当部分
- 使用した Bitwarden item 名 (`terminal-api-token` など)。実 token 値は記録しない

## シナリオ 3: 未登録カード

Traceability:

- `docs/spec/01_nfc_and_punch.md` の「未登録カードの取扱」
- `docs/spec/05_audit_and_backup.md` の「監査ログ確認」

### 環境前提

- PaSoRi RC-S380 が接続済みである
- カード B がどの従業員にも紐付いていない
- Server と Terminal がオンライン状態である

### 操作

1. Terminal を起動する
2. カード B を PaSoRi RC-S380 の中央に 1 枚だけ置く
3. Terminal の未登録カード表示を確認する
4. Admin 画面または監査記録を確認する
5. 打刻一覧にカード B 由来の打刻が作成されていないことを確認する

### 確認方法

- Terminal 画面で未登録表示を見る
- Admin 画面または DB / log で監査記録を確認する
- server log で未登録カード検出と非同期通知の発火対象を確認する

### 合格判定

- Terminal に未登録カードの案内が表示される
- 打刻は保存されない
- 未登録カード検出に対応する記録が残る
- Terminal 側で従業員選択ダイアログが表示されない

### 証跡

- Terminal の未登録表示
- 監査記録または log
- Admin 打刻一覧で新規打刻が増えていないことが分かる画面

## シナリオ 4: オフライン打刻と復旧同期

Traceability:

- `docs/spec/01_nfc_and_punch.md` の「オフライン時 (Server 停止 / ネットワーク遮断)」
- `docs/spec/01_nfc_and_punch.md` の「打刻フロー」
- `docs/spec/07_security.md` の「Terminal → Server (API)」

### 環境前提

- PaSoRi RC-S380 が接続済みである
- カード A が登録済み従業員に紐付いている
- Terminal は直近の従業員情報と履歴を local cache から表示できる状態である
- Server 停止と再起動を検証者が制御できる
- Admin 画面で復旧後の打刻一覧を確認できる
- Terminal 起動時に使用する Bitwarden item 名を記録できる

### 操作前記録

1. 実施日時、実施者、対象 commit hash を記録する
2. 使用するカード A の表示名または従業員名を記録する
3. Server 起動前後で実 token 値が log に出ていないことを確認する
4. Admin 画面でカード A の直近打刻件数と最後の打刻時刻を記録する
5. Terminal がオンライン待受状態であることを記録する

### 操作

1. Server と Terminal を起動する
2. Terminal が正常に接続できていることを確認する
3. Server を停止する、または Terminal から Server への通信を遮断する
4. Terminal が local cache モードまたはオフライン状態を表示することを確認する
5. カード A を PaSoRi RC-S380 の中央に 1 枚だけ置く
6. Terminal の確認 UI で従業員名、推定種別、直近履歴が local cache 由来で表示されることを確認する
7. OK 長押し、または 30 秒カウントダウンで確定する
8. Terminal 側で pending sync の打刻として保存されたことを確認する
9. 同じ pending 打刻を重複送信しない確認のため、Server 停止中に Terminal を再起動できる場合は再起動し、pending が残っていることを確認する
10. Server を再起動する、または通信を復旧する
11. Terminal が 30 秒間隔の health check で再接続を検知するまで待つ
12. Terminal が pending sync を古い順に送信することを terminal log で確認する
13. Admin 画面でカード A の打刻一覧を確認する
14. 同じ `punch_id` が 1 件だけ登録されていることを確認する
15. Terminal 側で pending 打刻が successfully_synced 相当に更新されたことを確認する

### 確認方法

- Terminal 画面を見る
- terminal log で local cache / retry sync の動きを確認する
- Admin 画面で打刻一覧を確認する
- server log で復旧後の `POST /api/punches` と応答を確認する
- 必要なら DB で `punch_id` 単位の件数を確認する
- 409 が発生した場合は、同一 `punch_id` の既存打刻として扱われ、重複が増えていないことを確認する

### 合格判定

- Server 停止中でも Terminal 側で打刻操作を完了できる
- オフライン中の打刻は pending sync として local cache に残る
- 復旧後に打刻が失われず同期される
- 同一打刻が重複登録されない
- 同期後に Admin 画面または監査記録から確認できる
- 同期時も Bearer token は Authorization header で送られ、URL query や log に token 実値が残らない
- Terminal の pending 状態が同期済みに更新される

### 証跡

- 操作前の Admin 打刻一覧または件数
- Server 停止中の Terminal 画面
- pending sync 保存を示す terminal log
- Server 復旧または通信復旧の時刻
- retry sync 開始と成功を示す terminal log
- server log の `POST /api/punches` 応答
- 復旧後の Admin 画面
- 同一 `punch_id` が 1 件だけであることを示す画面、log、または DB 確認結果
- 使用した Bitwarden item 名。実 token 値は記録しない

## シナリオ 5: 時刻ずれによる打刻停止

### 操作

1. Terminal と Server の時刻差が 10 秒を超える状態を作る
2. Terminal 画面を確認する
3. 可能ならカードをかざして挙動を見る
4. 時刻差を戻す
5. 再度 Terminal 画面を確認する

### 確認方法

- Terminal のエラー表示を見る
- 必要なら terminal log を確認する

### 合格判定

- 許容差超過時に打刻画面が無効化される
- 時刻同期エラーが表示される
- 時刻差を戻した後に通常利用へ復帰する

### 証跡

- エラー表示画面
- 復帰後画面

## シナリオ 6: LINE WORKS 紐付け済みユーザーの照会

### 操作

1. Server を `scripts/bw-run-server.sh` で起動する
2. LINE WORKS callback が有効であることを確認する
3. 紐付け済みユーザーから照会コマンドを送る
4. 返信メッセージを確認する

### 確認方法

- LINE WORKS クライアントで送受信を見る
- server log で callback 受信と user 解決を確認する

### 合格判定

- callback が受信される
- 正しいユーザーに解決される
- 照会結果の返信が届く

### 証跡

- LINE WORKS の送信画面
- LINE WORKS の返信画面
- server log の該当部分

## シナリオ 7: LINE WORKS 未紐付けユーザー

### 操作

1. 未紐付けユーザーから照会または修正コマンドを送る
2. 返信メッセージを確認する

### 確認方法

- LINE WORKS クライアントを確認する
- server log を確認する

### 合格判定

- 勤怠やシフトの実データは返らない
- 紐付け案内または未登録案内が返る

### 証跡

- LINE WORKS の返信画面
- server log の該当部分

## シナリオ 8: LINE WORKS 修正申請

### 操作

1. 紐付け済みユーザーで軽微修正を送る
2. 自動承認されるか確認する
3. 別途、過去日または大きい差分の修正を送る
4. Admin 画面で申請状態を確認する
5. 必要なら承認または却下する
6. LINE WORKS 返信を確認する

### 確認方法

- LINE WORKS クライアント
- Admin 画面
- server log
- 必要なら DB

### 合格判定

- 当日軽微修正は自動承認条件に従って反映される
- 要承認ケースは `requested` として残る
- Admin 操作後に状態遷移と返信が一致する

### 証跡

- LINE WORKS の送受信画面
- Admin 画面の申請状態
- 必要なら punch / request の確認結果

## 実施結果

| シナリオ | 実施日 | 実施者 | 対象 commit | 結果 | 備考 |
|---|---|---|---|---|---|
| Admin ログイン |  |  |  |  |  |
| PaSoRi RC-S380 登録済みカード打刻 |  |  |  |  |  |
| 未登録カード |  |  |  |  |  |
| オフライン打刻と復旧同期 |  |  |  |  |  |
| 時刻ずれによる打刻停止 |  |  |  |  |  |
| LINE WORKS 紐付け済み照会 |  |  |  |  |  |
| LINE WORKS 未紐付けユーザー |  |  |  |  |  |
| LINE WORKS 修正申請 |  |  |  |  |  |
