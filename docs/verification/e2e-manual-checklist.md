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
- `docs/spec/04_lineworks.md`
  - LINE WORKS callback
  - 照会・修正申請
  - 自動承認 / 要承認フロー
- `docs/spec/05_audit_and_backup.md`
  - 監査ログ確認
  - NTP 運用
- `docs/spec/07_security.md`
  - Admin ログイン
  - Terminal token
  - Bitwarden 運用
  - LINE WORKS 署名検証前提

## 実施環境

- Server を起動できるローカル環境
- Tauri terminal を起動できるローカル環境
- PaSoRi RC-S380
- 登録済みカード 1 枚以上
- 未登録カード 1 枚以上
- Admin アカウント
- Terminal API token
- LINE WORKS 検証用ユーザー
  - 紐付け済みユーザー
  - 未紐付けユーザー

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
- Pass / Fail
- server log の該当部分
- terminal log の該当部分
- 必要な画面スクリーンショット
- 必要なら DB 確認結果

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

## シナリオ 2: 登録済みカードの実機打刻 happy path

### 操作

1. Server を起動する
2. Terminal を起動する
3. Admin 画面にログインする
4. 登録済みカードを Terminal にかざす
5. Terminal の確認 UI を確認する
6. 30 秒待つ、または OK 長押しで確定する
7. Admin 画面の打刻一覧を確認する
8. 同じカードを再度かざし、推定種別の反転を確認する

### 確認方法

- Terminal 画面で従業員名、時刻、推定種別を見る
- Admin 画面で打刻一覧を見る
- 必要なら server log / terminal log を見る

### 合格判定

- Terminal に登録済み従業員として表示される
- 確認 UI に推定種別が表示される
- 確定後に打刻完了表示になる
- Admin 画面に打刻が 1 件追加される
- 次回スキャン時に推定種別が期待どおり反転する

### 証跡

- Terminal の確認 UI
- Terminal の打刻完了画面
- Admin の打刻一覧画面

## シナリオ 3: 未登録カード

### 操作

1. Terminal を起動する
2. 未登録カードをかざす
3. Admin 画面または監査記録を確認する

### 確認方法

- Terminal 画面で未登録表示を見る
- Admin 画面または DB / log で監査記録を確認する

### 合格判定

- Terminal に未登録カードの案内が表示される
- 打刻は保存されない
- 未登録カード検出に対応する記録が残る

### 証跡

- Terminal の未登録表示
- 監査記録または log

## シナリオ 4: オフライン打刻と復旧同期

### 操作

1. Server と Terminal を起動する
2. Terminal が正常に接続できていることを確認する
3. Server を停止する
4. 登録済みカードをかざす
5. Terminal 側で打刻完了相当の表示を確認する
6. Server を再起動する
7. 同期間隔経過後、Admin 画面で打刻一覧を確認する
8. 重複登録がないことを確認する

### 確認方法

- Terminal 画面を見る
- terminal log で local cache / retry sync の動きを確認する
- Admin 画面で打刻一覧を確認する
- 必要なら DB で件数確認する

### 合格判定

- Server 停止中でも Terminal 側で打刻操作を完了できる
- 復旧後に打刻が失われず同期される
- 同一打刻が重複登録されない
- 同期後に Admin 画面または監査記録から確認できる

### 証跡

- Server 停止中の Terminal 画面
- 復旧後の Admin 画面
- terminal log の sync 成功部分

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
| 登録済みカード打刻 |  |  |  |  |  |
| 未登録カード |  |  |  |  |  |
| オフライン打刻と復旧同期 |  |  |  |  |  |
| 時刻ずれによる打刻停止 |  |  |  |  |  |
| LINE WORKS 紐付け済み照会 |  |  |  |  |  |
| LINE WORKS 未紐付けユーザー |  |  |  |  |  |
| LINE WORKS 修正申請 |  |  |  |  |  |
