# 詳細仕様: 認証・セキュリティ

## スコープ

- 認証方式 (Admin / Terminal)
- シークレット管理 (Bitwarden 運用)
- Cloudflare Tunnel による外部公開
- パスワードポリシー
- LINE WORKS 署名検証

## 認証

### Admin → Server (Web UI)

- **Session + Cookie (HttpOnly, Secure, SameSite=Strict)**
- Session は `admin_session` テーブルに保存
- Cookie 名: `admin_session`
- 有効期限: 24 時間、活動時は自動延長 (last_active_at を更新)
- パスワードは `admin_user.password_hash` に **Argon2id** で保存
- Argon2id パラメータ: OWASP 推奨 (`t=2, m=19456 KiB, p=1`)
- ログイン失敗は `audit_log` に記録 (IP, UA も)
- 5 回連続失敗で 15 分アカウントロック

### Terminal → Server (API)

- **API Token (Bearer 方式)**
- Token は Admin Web の「Terminal 管理」画面から発行
- Token の生成: `base64url(random_bytes(32))` = 約 43 文字
- Server 側は `terminal.api_token_hash` に Argon2id ハッシュで保存
- Terminal 側は `config.toml` に**暗号化して保存**
  - 平文保存は禁止
  - ファイルパーミッション 0600
- Token ローテーション可能 (古い token を無効化)
- Token リボケーション可能 (Terminal を無効化)

```http
POST /api/punches
Authorization: Bearer <token>
Content-Type: application/json
```

### LINE WORKS Bot → Server (callback)

- **HMAC-SHA256 署名検証**
- Header: `X-WORKS-Signature`
- Bot Secret は Bitwarden から起動時注入
- **定数時間比較** (`subtle` crate) を必須
- 署名失敗は 401 + `audit_log` 記録

## シークレット管理

### 方針

- **Bitwarden CLI (`bw get`)** を使用
- AI エージェントには**実シークレットを見せない**
- 設定ファイルにはリファレンスのみ記載 (もしくは環境変数参照)

### 起動スクリプトパターン

```bash
#!/bin/bash
# /usr/local/bin/bw-run-server
set -euo pipefail

# Bitwarden のマスターパスワードは systemd の EnvironmentFile 経由
# (または TPM / age 等で暗号化して保存)
: "${BW_MASTER_PASSWORD:?BW_MASTER_PASSWORD must be set}"

export BW_SESSION=$(bw unlock --raw --passwordenv BW_MASTER_PASSWORD)

# Server が必要とする secrets を環境変数に展開
export LINEWORKS_BOT_SECRET=$(bw get password lineworks-bot-secret)
export LINEWORKS_API_TOKEN=$(bw get password lineworks-api-token)
export LINEWORKS_BOT_ID=$(bw get password lineworks-bot-id)
export DATABASE_ENCRYPTION_KEY=$(bw get password db-encryption-key)  # 将来用

bw lock > /dev/null

exec /usr/local/bin/pasori-timecard-server
```

### 設定ファイルでの記述

TOML は Bitwarden アイテム名を参照するだけ。実値は起動時に環境変数へ。

```toml
# /etc/pasori-timecard/server.toml
[database]
path = "/var/lib/pasori-timecard/attendance.db"

[lineworks]
bot_id_env = "LINEWORKS_BOT_ID"
bot_secret_env = "LINEWORKS_BOT_SECRET"
api_token_env = "LINEWORKS_API_TOKEN"
callback_path = "/api/lineworks/callback"

[server]
listen = "127.0.0.1:8080"
log_level = "info"
```

### 絶対にやってはいけない

- 設定ファイル / ソースコード / コミット履歴に平文 secret を残す
- `.env` ファイルを git にコミットする
- ログに secret を出力する (`tracing` のフィルタで明示的に除外)
- Docker イメージに secret を埋め込む (起動時注入のみ)

## Cloudflare Tunnel

### セットアップ手順

```bash
# Raspberry Pi 上で (ARM64 想定)
curl -L https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-arm64.deb \
    -o cloudflared.deb
sudo dpkg -i cloudflared.deb

# Cloudflare アカウントで認証 (ブラウザが開く)
cloudflared tunnel login

# Tunnel 作成
cloudflared tunnel create pasori-timecard

# DNS ルーティング (本番は独自ドメイン必須、開発/検証は `trycloudflare.com` 可)
cloudflared tunnel route dns pasori-timecard attendance.example.com

# 設定ファイル: ~/.cloudflared/config.yml
```

### config.yml

```yaml
tunnel: pasori-timecard
credentials-file: /home/pi/.cloudflared/abc-def-ghi.json

ingress:
  - hostname: attendance.example.com
    service: http://localhost:8080
  - service: http_status:404
```

### 起動

```bash
# systemd サービスとして起動
sudo cloudflared service install
sudo systemctl enable cloudflared
sudo systemctl start cloudflared
```

### LINE WORKS Bot 設定

- Bot 管理画面で Callback URL に `https://attendance.example.com/api/lineworks/callback` を登録

## パスワードポリシー

### Admin 管理者

- 最低 12 文字
- 英字 + 数字 + 記号を含む (任意だが推奨)
- 過去 3 世代のパスワードの再利用禁止 (v1.1 検討)
- パスワード変更時は確認用に現パスワード要求

### Terminal token

- `base64url(random_bytes(32))` = 約 256 bit
- 長さ / 複雑さはユーザー指定不可 (完全ランダム)

## Admin Web の CSRF 対策

- SameSite=Strict Cookie で防御
- すべての state-changing request は POST / PUT / DELETE
- 追加で CSRF token を発行・検証 (post-MVP で強化)

## 監査と通知

- ログイン失敗 5 回で LINE WORKS 管理者通知
- Terminal の不正 token 使用検出で LINE WORKS 管理者通知
- LINE WORKS 署名検証失敗で audit_log に記録

## 禁止事項

- 平文パスワード保存
- 平文 secret のコード / 設定ファイル混入
- `==` による token/signature 比較 (タイミング攻撃脆弱性)
- API token を URL クエリで送る (必ず Authorization header)
- Session Cookie を非 HttpOnly で発行する
- LINE WORKS callback の署名検証スキップ
