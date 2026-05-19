# 詳細仕様: 概要

このディレクトリは `AGENTS.md` から分離された**詳細仕様**を章ごとに格納する。
AGENTS.md は短く決定的に保つため、ここにボリュームを逃がす。

## 章立て

| ファイル | 内容 |
|---|---|
| `overview.md` (このファイル) | 全体俯瞰、ユーザー、ユースケース |
| `01_nfc_and_punch.md` | NFC 読取、打刻、確認 UI、推定ロジック |
| `02_attendance.md` | 勤怠表、集計、締め日、打刻修正 |
| `03_shift.md` | シフト表、差分検出 |
| `04_lineworks.md` | LINE WORKS 送受信、コマンド、承認フロー |
| `05_audit_and_backup.md` | 監査ログ、バックアップ、NTP、運用 |
| `06_data_model.md` | エンティティ / テーブル定義詳細 |
| `07_security.md` | 認証、シークレット、Cloudflare Tunnel |

## ユーザー

### 従業員 (Employee)
- 打刻端末でカードをかざす
- 確認 UI で種別を確認 / 変更
- LINE WORKS で自分の勤怠 / シフトを照会 (Phase 2)
- LINE WORKS で打刻漏れ・修正を申請 (Phase 2)

### 管理者 (AdminUser)
- Admin Web で従業員 / カード / 打刻 / シフトを管理
- LINE WORKS からの申請を承認 / 却下
- 監査ログを確認
- 通知設定 / 締め日 / NTP 設定を変更

### 打刻端末 (Terminal)
- 院内に固定設置された PC + PaSoRi
- キオスクモードで Tauri アプリ稼働
- Server に常時接続 (障害時は local cache)

### 院内サーバー (Server)
- Raspberry Pi 等の常時稼働マシン
- Cloudflare Tunnel で LINE WORKS callback 受信
- 管理者は LAN 内 / 院外 (Tunnel 経由) から Web UI にアクセス

## ユースケース (高レベル)

### UC-01: 出勤打刻
1. 従業員がカードをかざす
2. Terminal が NFC 読取、Server に問い合わせ
3. Server が従業員と直近打刻履歴を返す
4. Terminal が推定種別 (ClockIn) を含む確認 UI を表示
5. 30 秒カウントダウン開始、または OK 長押しでスキップ
6. 確定後、Terminal が punch_id (UUID v7) を生成して Server に POST
7. Server が audit log に記録、必要に応じて LINE WORKS 送信

### UC-02: 未登録カード
1. 従業員が未登録カードをかざす
2. Terminal が Server に問い合わせ
3. Server が「未登録」を返し、audit log に記録する
4. Terminal が Server から有効従業員一覧を取得する
5. 操作者が従業員を選択し、確認する
6. Terminal が Terminal API token でカード紐付け API を呼ぶ
7. Server が `card.bind` を audit log に記録する
8. Terminal は「山田太郎に登録しました」と表示し、打刻せず待受に戻る

### UC-03: LINE WORKS 修正申請 (Phase 2)
1. 従業員が LINE WORKS で「修正 2026-04-16 出勤 08:32」と送信
2. Server が callback 受信、署名検証、コマンド解釈
3. ルール適合 (当日中の軽微修正) なら自動承認、DB 反映、監査ログ記録
4. ルール外 (過去日 / 締め済み期間) なら requested 状態で保留、管理者に通知
5. 管理者が Admin Web で承認 / 却下
6. LINE WORKS に結果を返信

### UC-04: オフライン打刻
1. Server 停止中、Terminal がカードスキャン
2. Terminal が自身の local cache SQLite に保存 (status = `pending_sync`)
3. Server 復旧を検知 (定期 health check)
4. Terminal が pending_sync 打刻をまとめて Server に POST
5. UUID v7 の UNIQUE 制約で重複は自然に弾かれる

## データフロー

```
[NFC Card]
    │ scan
    ▼
[Terminal (Tauri)] ──(API Token, Bearer)──▶ [Server (axum)] ──▶ [SQLite (正本)]
        │                                           │
        │ local cache (オフライン時)                 ├──▶ [LINE WORKS 送信 (outbound)]
        ▼                                           │
  [Local SQLite]                                    └──◀── [LINE WORKS callback (Cloudflare Tunnel)]

[Admin Browser] ──(Session Cookie)──▶ [Server Admin API]
```

## 時刻フロー

- 保存・表示すべて Asia/Tokyo aware (`jiff::Zoned`)
- Terminal / Server とも NTP 同期必須 (±10 秒)
- Terminal オンライン打刻: Server の `recorded_at` を正とする、Terminal の `occurred_at` も記録
- Terminal オフライン打刻: Terminal の `occurred_at` を記録、再送時に `source = local_cached` フラグ
