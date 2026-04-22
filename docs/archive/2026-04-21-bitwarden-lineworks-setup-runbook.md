# Bitwarden CLI / LINE WORKS Bot セットアップ手順 (2026-04-21)

## 目的

- 本プロジェクトで必要な secret を Bitwarden CLI 経由で安全に注入する
- LINE WORKS Bot を作成し、院内サーバーから callback / 通知送信できる状態にする
- 実装者がローカル調査や実機検証を再現できるようにする

この文書は **運用手順書 (runbook)** である。プロダクト仕様の確定事項は
`AGENTS.md` と `docs/spec/` を正とする。

## 前提

- Bitwarden アカウントを利用できる
- LINE WORKS の管理者権限、または Developer Console で Bot を登録できる権限がある
- callback を受ける URL を公開できる
  - 開発: `trycloudflare.com` など一時 URL でも可
  - 運用: 独自ドメイン + Cloudflare Tunnel 前提

## 1. Bitwarden CLI のセットアップ

### 1.1 インストール

Bitwarden 公式 CLI:

- 公式: <https://bitwarden.com/help/cli/>

インストール後の確認:

```bash
bw --version
bw --help
```

### 1.2 ログイン方法

自動化やサーバー運用では、公式上も `bw login --apikey` が適している。

公式で確認できる事項:

- `bw login --apikey`
- API key 用環境変数:
  - `BW_CLIENTID`
  - `BW_CLIENTSECRET`
- vault を使うには、その後に `bw unlock` が必要

初回ログイン例:

```bash
export BW_CLIENTID="..."
export BW_CLIENTSECRET="..."
bw login --apikey
```

### 1.3 unlock と session

公式で確認できる事項:

- `bw unlock`
- `bw unlock --passwordenv <ENV_NAME>`
- unlock 後は `BW_SESSION` を export して使う

本プロジェクト向けの推奨例:

```bash
export BW_MASTER_PASSWORD="..."
export BW_SESSION="$(bw unlock --passwordenv BW_MASTER_PASSWORD --raw)"
```

補足:

- `--passwordenv` は公式 documented
- `BW_SESSION` は terminal ごとに保持される
- 作業後は `bw lock` または `bw logout` を実行する

### 1.4 secret の取り出し

本プロジェクトで必要な最小 secret:

- `lineworks-bot-id`
- `lineworks-bot-secret`
- `lineworks-api-token`
- 必要なら `lineworks-admin-channel-id`

取得例:

```bash
export LINEWORKS_BOT_ID="$(bw get password lineworks-bot-id)"
export LINEWORKS_BOT_SECRET="$(bw get password lineworks-bot-secret)"
export LINEWORKS_API_TOKEN="$(bw get password lineworks-api-token)"
export LINEWORKS_ADMIN_CHANNEL_ID="$(bw get password lineworks-admin-channel-id)"
```

### 1.5 起動ラッパー例

`docs/spec/05_audit_and_backup.md` / `docs/spec/07_security.md` の方針に沿う例:

```bash
#!/bin/bash
set -euo pipefail

: "${BW_MASTER_PASSWORD:?BW_MASTER_PASSWORD must be set}"

export BW_SESSION="$(bw unlock --passwordenv BW_MASTER_PASSWORD --raw)"
export LINEWORKS_BOT_ID="$(bw get password lineworks-bot-id)"
export LINEWORKS_BOT_SECRET="$(bw get password lineworks-bot-secret)"
export LINEWORKS_API_TOKEN="$(bw get password lineworks-api-token)"
export LINEWORKS_ADMIN_CHANNEL_ID="$(bw get password lineworks-admin-channel-id)"

exec cargo run -p server
```

### 1.6 禁止事項

- `.env` を git に commit しない
- secret を `docs/` や source code に直接書かない
- `echo $LINEWORKS_API_TOKEN` のような確認結果をスクリーンショットに残さない

## 2. LINE WORKS Bot 作成の流れ

### 2.1 確認できた公式ソース

- Bot 管理画面:
  - <https://help.worksmobile.com/ja/admin-guides/manage-service/bot/>
- Bot トークルーム FAQ:
  - <https://help.worksmobile.com/ja/faqs/services/line-works/message/others/what-is-bot-message-room/>
- 開発者向け公式サンプル:
  - <https://github.com/lineworks/works-api-code-samples>
  - <https://github.com/lineworks/samplebot_attendance_management_bot_v1.0>

### 2.2 作成の全体像

1. Developer Console で Bot を登録する
2. Admin 画面で Bot を追加し、利用権限と公開範囲を設定する
3. callback URL と secret / token / bot id をサーバー側に設定する
4. Bot を 1:1 またはメッセージルームで利用開始する

### 2.3 Developer Console 側

今回の調査で admin help から確認できたのは、
**Developer Console で登録した Bot を Admin 画面で利用可能化する** という流れまでである。

したがって、実務上の手順は以下のように扱う。

1. LINE WORKS Developer Console を開く
2. Bot を新規登録する
3. Bot 名、説明、担当者、必要な会話タイプを設定する
4. callback を使う場合は callback URL を設定する
5. Bot ID / Bot Secret / API Token を発行または確認する

注意:

- Developer Console の細かな項目名やボタン名は、今回の取得環境では公式本文を
  安定参照できていない
- そのため、画面ラベルは **実画面優先** で確認すること

### 2.4 Admin 画面での Bot 追加

公式 help で確認できた手順:

1. Admin 左メニューで `サービス` → `Bot`
2. `Bot追加`
3. Developer Console に登録済みの Bot を選んで追加

追加後に設定する項目:

- 使用権限
  - `すべて`
  - `メンバー指定`
- 公開設定
  - `公開`
  - `非公開`

公式 help で確認できた制約:

- 1:N メッセージルーム / 組織・グループメッセージルームで会話できるタイプの Bot は
  `すべて` 権限のみ選択可能
- 1:1 会話のみの Bot は `指定メンバー` を選べる

### 2.5 1:1 とメッセージルーム

FAQ で確認できた事項:

- Bot タイプに応じて 1:1 のみ、またはチーム / グループ / 1:N トークルームにも参加可能
- 初回の 1:1 対話では `はじめる` ボタンで開始する場合がある

このプロジェクトで必要な前提:

- 本人照会 / 本人修正申請:
  - 1:1 対話が必要
- 管理者向け一斉通知:
  - 管理者用 room / channel に Bot を参加させる運用が必要

## 3. 本プロジェクト用の LINE WORKS 設定値

### 3.1 最低限必要な環境変数

サーバー側:

```bash
LINEWORKS_BOT_ID
LINEWORKS_BOT_SECRET
LINEWORKS_API_TOKEN
```

管理者 room 通知まで使う場合:

```bash
LINEWORKS_ADMIN_CHANNEL_ID
```

### 3.2 callback URL

本プロジェクトの callback path:

```text
/api/lineworks/callback
```

開発例:

```text
https://<temporary-public-host>/api/lineworks/callback
```

運用例:

```text
https://attendance.example.com/api/lineworks/callback
```

### 3.3 callback の確認項目

- Bot Secret がサーバーに入っていること
- `X-WORKS-Signature` 検証が通ること
- unknown command でも 500 にしないこと
- 本人の `external_user_id` が `external_account` に紐付いていること

## 4. 実機検証手順

### 4.1 user 宛て送信確認

公式 sample で根拠が取れている範囲:

- endpoint:
  `https://www.worksapis.com/v1.0/bots/{botId}/users/{userId}/messages`
- header:
  `Authorization: Bearer <token>`
- payload:

```json
{
  "content": {
    "type": "text",
    "text": "Hello"
  }
}
```

確認用 curl:

```bash
curl -sS -X POST \
  "https://www.worksapis.com/v1.0/bots/${LINEWORKS_BOT_ID}/users/${LINEWORKS_TEST_USER_ID}/messages" \
  -H "Authorization: Bearer ${LINEWORKS_API_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{
    "content": {
      "type": "text",
      "text": "pasori_timecard_v2 test"
    }
  }'
```

### 4.2 channel 宛て送信確認

`docs/archive/2026-04-21-lineworks-official-source-check.md` の通り、
2026-04-21 時点では channel 宛て送信の公式 sample 本文は未取得である。

それでも検証する場合は、spec 採用中の endpoint で実測する。

```bash
curl -sS -X POST \
  "https://www.worksapis.com/v1.0/bots/${LINEWORKS_BOT_ID}/channels/${LINEWORKS_ADMIN_CHANNEL_ID}/messages" \
  -H "Authorization: Bearer ${LINEWORKS_API_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{
    "content": {
      "type": "text",
      "text": "pasori_timecard_v2 channel test"
    }
  }'
```

記録すべき項目:

- request URL
- request payload
- HTTP status
- response body
- Bot が対象 room/channel に参加済みか

## 5. 運用上のメモ

### 5.1 管理者向け通知 room

本プロジェクトでは `LINEWORKS_ADMIN_CHANNEL_ID` を別管理する方がよい。

理由:

- 未登録カード通知
- 日次締め結果
- 管理者修正反映

などは個人宛てではなく、管理者 room への通知が自然だから。

### 5.2 本人 user ID の扱い

`external_account.external_user_id` を user 宛て送信先 ID として使う設計を採っている。

ただし、これは公式 sample の `LW_API_20_USER_ID` 例と整合する一方で、
API リファレンス本文での最終確認は未了である。

そのため、実機検証結果を `docs/archive/2026-04-21-lineworks-official-source-check.md`
へ追記して補強すること。

## 参考

- Bitwarden CLI:
  - <https://bitwarden.com/help/cli/>
- LINE WORKS Admin Bot:
  - <https://help.worksmobile.com/ja/admin-guides/manage-service/bot/>
- LINE WORKS Bot トークルーム FAQ:
  - <https://help.worksmobile.com/ja/faqs/services/line-works/message/others/what-is-bot-message-room/>
- LINE WORKS API samples:
  - <https://github.com/lineworks/works-api-code-samples>
- LINE WORKS attendance bot sample:
  - <https://github.com/lineworks/samplebot_attendance_management_bot_v1.0>
- 関連調査メモ:
  - [2026-04-21-lineworks-official-source-check.md](./2026-04-21-lineworks-official-source-check.md)
