# LINE WORKS 公式ソース照合メモ (2026-04-21)

## 目的

- `crates/server/src/infra/lineworks_notify.rs`
- `crates/server/src/lineworks.rs`
- `crates/core/src/application/lineworks.rs`

の実装を進めるにあたり、LINE WORKS 公式ソースと現行仕様の一致点・未確認点を
整理する。

この文書は **調査メモ** であり、`docs/spec/04_lineworks.md` の確定仕様を置き換える
ものではない。仕様変更が必要な場合は ADR を追加してから `docs/spec/` を更新する。

## 調査日

- 2026-04-21

## 確認できた公式ソース

### 1. LINE WORKS 公式 help: Message Bot

- URL: <https://help.worksmobile.com/en/use-guides/message/bot/message-bot/>
- 確認できたこと:
  - Bot をメッセージルームへ招待して利用する運用が存在する
  - Bot は 1:1 だけでなくメッセージルームでも運用できる

### 2. LINE WORKS 公式 GitHub: works-api-code-samples

- URL: <https://github.com/lineworks/works-api-code-samples>
- 確認できたこと:
  - LINE WORKS 公式が API / Bot サンプル集を公開している
  - `bot-send-message` サンプルが存在する
  - 検索エンジン経由で確認できた公開サンプル一覧は次の 4 件だった
    - `bot-send-message`
    - `bot-echo-fastapi`
    - `bot-echo-express`
    - `bot-echo-spring-boot`
  - この取得範囲では channel 宛て送信専用の公開サンプルは見つかっていない

#### 2.1 `bot-send-message` サンプルから直接確認できた事項

- リポジトリ上のサンプルパス:
  - `samples/python/bot-send-message/README.md`
  - `samples/python/bot-send-message/main.py`
- raw 参照 URL:
  - <https://raw.githubusercontent.com/lineworks/works-api-code-samples/main/samples/python/bot-send-message/README.md>
  - <https://raw.githubusercontent.com/lineworks/works-api-code-samples/main/samples/python/bot-send-message/main.py>
- GitHub 上で確認できた内容:
  - 環境変数として `LW_API_20_BOT_ID` と `LW_API_20_USER_ID` を受け取る
  - `LW_API_20_USER_ID` の README 例は `cccc@bbbb`
  - OAuth token endpoint は `https://auth.worksmobile.com/oauth2/v2.0/token`
  - API base URL は `https://www.worksapis.com/v1.0`
  - scope は `bot`
  - user 宛て送信 endpoint は
    `https://www.worksapis.com/v1.0/bots/{bot_id}/users/{user_id}/messages`
  - HTTP header は
    - `Content-Type: application/json`
    - `Authorization: Bearer {access_token}`
  - text message payload 例は以下

```json
{
  "content": {
    "type": "text",
    "text": "Hello"
  }
}
```

  - flex message 例も同じ `content` ルート配下で送っている

#### 2.2 `bot-send-message` からの推論

- `external_account.external_user_id` は、少なくとも公式サンプル上では
  `cccc@bbbb` のような ID を想定している
- ただしこれは README の環境変数例からの推論であり、
  **developers.worksmobile.com の API リファレンス本文で最終確認はまだできていない**
- また、公式サンプルは user 宛て送信のみで、channel 宛て送信の実例までは含んでいない

### 3. LINE WORKS 公式 GitHub: samplebot_attendance_management_bot_v1.0

- URL: <https://github.com/lineworks/samplebot_attendance_management_bot_v1.0>
- 確認できたこと:
  - 勤怠管理 bot の公開サンプルが存在する
  - 本プロジェクトの LINE WORKS 勤怠連携に近い参照元として妥当
  - README では管理者アカウントに接続された bot が共有カレンダーへ打刻時刻イベントを
    作るサンプルであると説明されている
  - GitHub の repo ページ上では、少なくとも以下のファイル / ディレクトリ構成を確認できた
    - `main.py`
    - `attendance_management_bot/`
    - `conf/`
    - `doc/`
    - `docs/`
  - README から、詳細解説ページへの公式リンクも確認できた
    - <https://pages.oss.navercorp.com/works-mobile/oneapp_samplebot_attendance_management_bot/index.html>
  - ただし今回の取得範囲では `main.py` 本文や詳細解説ページ本文までは安定して参照できず、
    送信 endpoint / payload の根拠としてはまだ使っていない

## 現時点で公式ソースと矛盾していない事項

### Bot による送受信を前提にすること

- 本プロジェクトの `docs/spec/04_lineworks.md` は Bot 前提で記述している
- これは LINE WORKS 公式 help の Message Bot 概要と矛盾しない

### user 宛て送信と room/channel 宛て送信を分ける設計

- `docs/spec/04_lineworks.md` では以下の 2 形式を想定している
  - `POST https://www.worksapis.com/v1.0/bots/{botId}/users/{userId}/messages`
  - `POST https://www.worksapis.com/v1.0/bots/{botId}/channels/{channelId}/messages`
- 送信先が個人とメッセージルームで分かれる前提は、公式 help の「Bot を
  メッセージルームへ招待できる」という運用と整合する

## まだ公式に直接確認できていない事項

### developers.worksmobile.com の API リファレンス本文

- `developers.worksmobile.com` はこの調査時点の取得環境では JavaScript 前提ページに
  当たりやすく、API リファレンス本文を直接抜けていない
- そのため、以下は **まだ公式リファレンス本文で直確認できていない**
  - `users/{userId}` / `channels/{channelId}` の request/response 形式
  - text message payload の厳密な JSON 形式
  - channel 宛てと user 宛てで必要な前提条件の差

### user ID の解決方法

- `external_account.external_user_id` をそのまま送信 API の `userId` に使ってよいかを
  公式リファレンス本文で未確認
- 現状の設計では `external_user_id` を LINE WORKS User ID として保持しているが、
  実装確定前に公式サンプルか公式リファレンスで照合する必要がある

### channel 宛て送信の payload 例

- 今回確認できた公式サンプルの範囲では、`channels/{channelId}/messages` を直接叩く
 送信サンプルまでは確認できていない
- したがって、現行実装の channel 宛て通知は
  **endpoint 形式は spec どおり維持しつつ、payload 互換性は未確認** と扱うべき
- この未確認は「公式ソースが存在しない」のではなく、
  **2026-04-21 のこの調査手段では該当サンプルまたは API 本文を捕捉できていない**
  という意味である

### 2026-04-21 時点の検索結果として残すべきこと

- `lineworks/works-api-code-samples` の公開一覧からは `bot-send-message` 以外に
  channel 宛て送信を明示したサンプルは確認できなかった
- `lineworks` 組織の公開 repo 一覧からは
  `samplebot_attendance_management_bot_v1.0` の存在と README までは確認できたが、
  channel 宛て送信 endpoint を根拠として抜ける `main.py` / 詳細解説本文までは
 取得できなかった
- したがって、2026-04-21 時点では
  **user 宛て送信は公式サンプルで裏付けあり / channel 宛て送信は spec 上の採用だが
  公式本文未確認**
  という整理が最も正確

## 実装上の含意

### すでに進めてよい範囲

- LINE WORKS callback の署名検証
- コマンドパース
- `attendance_request` の状態遷移
- 管理者承認 API
- 管理者向け channel 通知の文面構築

これらは本プロジェクト内 spec と公式公開情報の範囲で矛盾が見えていない。

### 公式再確認が必要な範囲

- 管理者向け channel 通知の endpoint と権限前提
- channel 宛て送信の payload 形式
- 本人向け通知で `external_account.external_user_id` を直接送信先に使う箇所の
  最終妥当性確認

### 公式サンプルで確認済みとして扱ってよい範囲

- OAuth token endpoint:
  `https://auth.worksmobile.com/oauth2/v2.0/token`
- Bot scope で access token を取り、`Bearer` で送信すること
- user 宛て送信 endpoint:
  `https://www.worksapis.com/v1.0/bots/{botId}/users/{userId}/messages`
- `content` 直下にメッセージ本体を置く JSON payload 形式
- `LW_API_20_USER_ID` の利用例が `cccc@bbbb` で示されていること

## 次にやるべき確認

1. `lineworks/works-api-code-samples` の `bot-send-message` 実装を読み、現行
   `lineworks_notify.rs` の endpoint / payload と一致するか照合する
2. `lineworks/samplebot_attendance_management_bot_v1.0` を読み、勤怠 bot での
   宛先解決や callback 取り扱いを比較する
3. developers.worksmobile.com の該当ページを別手段で開き、
   `channels/{channelId}/messages` の request body を確認する
4. channel 宛て送信の公式サンプルが見つからない場合は、その不在自体をメモに残した上で
   実機検証ログを補助根拠として追加する

## 判断

- この調査時点では、`LINE WORKS` 実装は「完全に公式 API リファレンス照合済み」とは
  言えない
- 一方で、Bot 利用前提、user/channel 送信分離前提、勤怠 bot を MVP に含める前提は
  公式公開情報と矛盾していない
- したがって、今後の実装は **管理者向け通知や承認フローは進めてよいが、送信 API の
  最終確定は公式サンプルまたは公式 API リファレンス本文を確認してから行う**
