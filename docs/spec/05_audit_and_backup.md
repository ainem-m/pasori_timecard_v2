# 詳細仕様: 監査・バックアップ・運用

## スコープ

- 監査ログ (audit_log) の対象と構造
- バックアップ戦略
- NTP 運用
- Cloudflare Tunnel
- systemd サービス設定

## 監査ログ

### 対象イベント (既定: 広範囲)

以下を `audit_log` の対象候補とする。設定で ON/OFF 可能で、既定は全 ON。

| カテゴリ | イベント |
|---|---|
| 打刻 | `punch.create_manual` / `punch.update` / `punch.soft_delete` |
| 従業員 | `employee.create` / `employee.update` / `employee.deactivate` |
| カード | `card.bind` / `card.unbind` / `card.rebind` |
| 管理者 | `admin.login_success` / `admin.login_failure` / `admin.password_change` |
| 設定 | `settings.update` (cutoff_date, ntp_tolerance, 他) |
| LINE WORKS | `request.auto_approved` / `request.approved` / `request.rejected` |
| その他 | `terminal.registered` / `terminal.token_rotated` |

### データ構造

| フィールド | 型 |
|---|---|
| id | UUID v7 |
| actor_type | `admin` / `employee` / `system` / `terminal` |
| actor_id | Option\<UUID\> |
| action | String (上記のドット区切り識別子) |
| target_type | String |
| target_id | Option\<UUID\> |
| before_json | Option\<JSON\> |
| after_json | Option\<JSON\> |
| metadata_json | Option\<JSON\> |
| created_at | Zoned |

### 追加禁止事項

- `UPDATE` / `DELETE` は **SQLite レベルで禁止** する
  - SQLite トリガーで `BEFORE UPDATE/DELETE ON audit_log RAISE ABORT` を設定
- 管理画面からも削除 UI は提供しない
- ログの手動編集はファイル監視で警告 (v1.1 検討)

## バックアップ

### 日次自動バックアップ

- Server 起動時、および毎日 03:00 (JST) に実行
- `cp attendance.db attendance.backup.YYYYMMDD.db` (SQLite の online backup API を使う)
- **過去 30 日分を保持**、古いものから自動削除

### 手動バックアップ

- Admin Web の「バックアップ」画面に「今すぐバックアップ」ボタン
- ダウンロードボタンでブラウザから保存も可能

### リストア

- Admin Web の「リストア」画面で過去 30 日分から選択
- 現在の DB は `attendance.before_restore.YYYYMMDDHHMMSS.db` として自動退避
- リストアは Server 再起動を要求

### バックアップファイルの保存場所

- Linux: `/var/lib/pasori-timecard/backups/`
- systemd で `StateDirectory=pasori-timecard` を指定

## NTP 運用

### Server 側

- Raspberry Pi は systemd-timesyncd で常時 NTP 同期
- 同期先は `time.google.com` / `ntp.nict.jp` 等を推奨
- 同期状態は `GET /api/health` のレスポンスに含める

### Terminal 側

- OS の NTP 機能を使用
- Terminal 起動時と 10 分ごとに同期状態チェック
- Server の時刻との差分も `GET /api/health` で検証
- 許容誤差 ±10 秒を超えたら打刻画面を無効化

## Cloudflare Tunnel

### 必要なもの (配布先に要求)

1. Cloudflare アカウント (無料枠で十分)
2. 本番は独自ドメイン必須。開発/検証のみ Cloudflare 提供の `*.trycloudflare.com` を許容
3. Raspberry Pi / 院内サーバーに `cloudflared` をインストール

### 設定手順 (運用ドキュメントに詳細、ここは要約)

```bash
# Raspberry Pi 上で
curl -L https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-arm64.deb -o cloudflared.deb
sudo dpkg -i cloudflared.deb
cloudflared tunnel login
cloudflared tunnel create pasori-timecard
cloudflared tunnel route dns pasori-timecard attendance.example.com
cloudflared tunnel run pasori-timecard
```

### ルーティング

- `attendance.example.com` → localhost:8080 (Server)
- LINE WORKS callback URL として Bot に設定するのは `https://attendance.example.com/api/lineworks/callback`

### セキュリティ

- Cloudflare Access で Admin Web に IP 制限や SSO を追加できる (post-MVP 検討)
- LINE WORKS callback は署名検証必須で既に保護されている

## systemd サービス

### pasori-timecard-server.service

```ini
[Unit]
Description=PaSoRi Timecard Server
After=network.target

[Service]
Type=simple
User=pasori
StateDirectory=pasori-timecard
WorkingDirectory=/var/lib/pasori-timecard
ExecStart=/usr/local/bin/bw-run-server
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
```

### bw-run-server (起動ラッパースクリプト)

```bash
#!/bin/bash
# Bitwarden から secrets を取得して環境変数に注入し、Server を起動
set -euo pipefail

export BW_SESSION=$(bw unlock --raw --passwordenv BW_MASTER_PASSWORD)
export LINEWORKS_BOT_SECRET=$(bw get password lineworks-bot-secret)
export LINEWORKS_API_TOKEN=$(bw get password lineworks-api-token)

exec /usr/local/bin/pasori-timecard-server
```

## ログ

### 保存場所

- Linux: `/var/log/pasori-timecard/`
- macOS: `~/Library/Logs/pasori-timecard/`
- Windows: `%LOCALAPPDATA%\pasori-timecard\logs\`

### ローテーション

- `tracing-appender` の `daily` rotation を使用
- 30 日分保持、古いものから自動削除

### レベル

- 既定: `INFO`
- エラー発生時: `WARN` / `ERROR`
- Admin Web から環境に応じて `DEBUG` / `TRACE` を一時的に有効化可能

### ログ閲覧

- Admin Web の「ログ」画面から直近 7 日分のログを閲覧
- ダウンロード (zip) も可能
- **個人情報** (従業員名、カード ID) はログに含めない。UUID のみで記録

## エラートラッキング (方針)

- MVP はローカルログのみ
- Sentry / GlitchTip は医療機関のプライバシー配慮で採用しない
- post-MVP で GlitchTip (Sentry 互換、自ホスト可) を Gateway 同居で検討
