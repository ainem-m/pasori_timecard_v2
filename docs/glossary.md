# Glossary (用語集)

コード内の識別子名、UI 上の日本語表記、概念の説明を**一意に固定**する。
迷ったらこのファイルに従うこと。新語を追加する場合は PR で追記する。

## ドメイン用語

| コード名 (Rust / TS) | UI 表記 | 説明 |
|---|---|---|
| `Employee` | 従業員 | 打刻対象者。`is_active=false` でも履歴参照のため残す。 |
| `Card` | ICカード | 物理 IC カード。`card_identifier` は FeliCa IDm。 |
| `CardId` | (内部) | `Card.card_identifier` の hex 文字列表現。 |
| `PunchEvent` | 打刻 | 1 件の出退勤記録。自動 (NFC) / 手動修正 / インポートのいずれかから生成される。 |
| `ClockIn` | 出勤 | 打刻種別の 1 つ。 |
| `ClockOut` | 退勤 | 打刻種別の 1 つ。 |
| `BreakStart` | 休憩開始 | **将来拡張**。MVP では未使用。 |
| `BreakEnd` | 休憩終了 | **将来拡張**。MVP では未使用。 |
| `TemporaryOut` | 外出 | **将来拡張**。MVP では未使用。 |
| `TemporaryReturn` | 戻り | **将来拡張**。MVP では未使用。 |
| `ManualCorrection` | 手動修正 | 管理者が作成した打刻 (`source = manual`)。 |
| `AttendanceDay` | 日次勤怠 | ある従業員の 1 日分。ペアリングと勤務時間を計算した結果。永続化しない (計算結果)。 |
| `MonthlyTimesheet` | 月次勤怠表 | 締め日ベースの 1 ヶ月分の集計。 |
| `ShiftType` | シフト種別 | 通常勤務 / 午前 / 午後 / 休み / 有休 / 特別休暇 / 予備枠。マスタ管理。 |
| `ShiftAssignment` | 勤務予定 | ある従業員・ある日付のシフト 1 件。`status` = `draft` / `published` / `finalized`。 |
| `AuditLog` | 監査ログ | append-only、DELETE 禁止。 |
| `AttendanceRequest` | 勤怠申請 | LINE WORKS 経由の修正申請・照会履歴。 |
| `ExternalAccount` | 外部アカウント | LINE WORKS の送信者 ID と `Employee` の紐付け。 |
| `AdminUser` | 管理者 | Admin Web にログインできるユーザー。 |
| `Terminal` | 打刻端末 | Tauri で動作する端末の登録情報。API token を持つ。 |

## 打刻フロー関連

| コード名 | UI 表記 | 説明 |
|---|---|---|
| `ReaderBackend` | (内部) | NFC 読取の抽象インターフェース。 |
| `ReaderStatus` | リーダー状態 | `Disconnected` / `Connecting` / `Ready` / `Error`。UI ではアイコンで表示。 |
| `CardScanned` | (内部) | NFC 読取成功イベント。 |
| `PunchPolicy` | (内部) | 次の打刻種別を推定するロジック。 |
| `DefaultPunchPolicy` | (内部) | v1 互換の既定ポリシー (前日またぎ + 同日反対)。 |
| `RoundingPolicy` | (内部) | 集計時の時刻丸めポリシー。 |
| `NoRounding` | (内部) | MVP 既定。素通し。 |

## 状態

| コード名 | UI 表記 | 説明 |
|---|---|---|
| `TerminalMode::Kiosk` | 端末モード | 打刻端末の通常状態。キオスク固定。 |
| `TerminalMode::Confirming` | 確認モード | 打刻直後の 30 秒猶予中。種別変更 / キャンセル可。 |
| `request_type = correction` | 修正申請 | LINE WORKS 経由の打刻修正リクエスト。 |
| `request_type = query` | 照会 | LINE WORKS 経由の勤怠・シフト照会。 |
| `status = requested` | 申請中 | 申請受理直後。 |
| `status = auto_approved` | 自動承認 | ルール適合で自動反映された。 |
| `status = approved` | 承認済み | 管理者が承認した。 |
| `status = rejected` | 却下 | 管理者が却下した。 |
| `status = applied` | 反映済み | DB に反映された。 |
| `status = cancelled` | 取り消し | 申請者が取り消した。 |

## コンポーネント

| コード名 | UI 表記 | 説明 |
|---|---|---|
| `terminal` crate | 打刻端末 | Tauri アプリ。キオスクで NFC と打刻 UI のみ。 |
| `server` crate | 院内サーバー | axum の HTTP サーバー。データ正本を持つ。 |
| `core` crate | コア | domain + application + infra traits。Terminal と Server が共有。 |
| `import_v1` crate | v1 インポーター | CLI ツール。v1 SQLite を v2 形式に取り込む。 |
| `web/admin` | 管理画面 | ブラウザから Server にアクセス。 |
| `web/terminal` | 端末 UI | Tauri 内で動作する React アプリ。 |

## 避ける語彙

以下は**混乱を招くので使わない**。

| ❌ 避ける | ✅ 使う |
|---|---|
| Worker / Staff | Employee |
| TimeRecord | PunchEvent |
| CheckIn / CheckOut | ClockIn / ClockOut |
| Timestamp | `occurred_at` (打刻発生時刻) or `recorded_at` (Server 受信時刻) |
| User | Employee (従業員) / AdminUser (管理者) — 曖昧な「User」は禁止 |
| Login | AdminLogin (管理者ログイン) — 従業員はログインしない |
