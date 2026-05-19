# 詳細仕様: LINE WORKS 連携

## スコープ

- 送信 (Server → LINE WORKS): 4 種類の通知
- 受信 (LINE WORKS → Server callback): 照会・修正申請
- 承認フロー
- Cloudflare Tunnel 経由の外部公開

## 送信 (outbound)

### 対象イベント (Phase 2)

LINE WORKS 連携は Phase 2 とする。MVP では設定が存在しない場合は完全に無効化し、
Admin / Terminal の主導線には表示しない。

| イベント | 送信先 | 内容 |
|---|---|---|
| 打刻漏れ疑い | 本人 + 管理者 | 「本日の退勤打刻が検出されていません」 |
| 未登録カード検出 | 管理者 | 「未登録カード (XXXX...) がスキャンされました」 |
| シフト公開 | 全対象従業員 | 「2026 年 5 月のシフトが公開されました」 |
| 給料概算配信 | 本人 | 月末に「今月の勤務時間: XX 時間、概算: YY 円」 |

### 実装

- `core::notify::Notifier` trait 経由で発火
- `infra_notify_lineworks::LineWorksNotifier` が具象実装
- **非同期 fire-and-forget**。`notify()` のエラーは打刻処理を失敗させない
- 失敗時はリトライ queue に積み、ログに WARN 出力
- LINE WORKS API: Bot token を `Authorization: Bearer` で送る

### API

- `POST https://www.worksapis.com/v1.0/bots/{botId}/users/{userId}/messages`
- `POST https://www.worksapis.com/v1.0/bots/{botId}/channels/{channelId}/messages`

## 受信 (inbound callback)

### エンドポイント

- `POST /api/lineworks/callback` (Server 側)
- Cloudflare Tunnel 経由で外部公開

### 署名検証 (必須)

- LINE WORKS は `X-WORKS-Signature` ヘッダに HMAC-SHA256 署名を付ける
- Server は Bot Secret (Bitwarden から取得) で検証
- 署名不一致は **401 Unauthorized** を返して処理を中断
- 検証失敗は audit_log に記録

### 署名検証コードの要件

```rust
// 擬似コード
fn verify_signature(body: &[u8], signature: &str, secret: &[u8]) -> bool {
    let expected = hmac_sha256(secret, body);
    let expected_b64 = base64::encode(&expected);
    constant_time_eq(signature.as_bytes(), expected_b64.as_bytes())
}
```

**定数時間比較** (`subtle` crate 等) を使うこと。通常の文字列比較は禁止。

## コマンド設計

### 優先順位 (UX)

1. **固定メニュー** (LINE WORKS の Rich Menu)
2. **Postback ボタン**
3. **定型コマンド** (文字列マッチング)
4. **自由入力** (最後の手段、正規表現でパース)

### 定型コマンド (MVP)

| コマンド例 | 動作 |
|---|---|
| `今日の勤怠` | 当日の出退勤時刻を返信 |
| `今月の勤怠` | 当月 (締め日基準) の勤務時間合計を返信 |
| `今日のシフト` | 当日の予定シフトを返信 |
| `今月のシフト` | 当月のカレンダー形式シフトを返信 |
| `出勤忘れ 08:30` | 当日の出勤打刻申請 |
| `退勤忘れ 18:05` | 当日の退勤打刻申請 |
| `修正 2026-04-16 出勤 08:32` | 指定日の出勤時刻修正申請 |
| `ヘルプ` | コマンド一覧 |

### パーサ方針

- 未知コマンドは `ヘルプ` に落とす
- 自由入力は panic せず、必ず解釈結果を返す
- 照会コマンドは DB 変更を伴わない自動応答として扱う

## 承認フロー

### 状態遷移

```
[LINE WORKS 受信]
       ▼
  [requested]
       │
       ├─[ルール適合 → 自動承認]─▶ [auto_approved] ─▶ [applied]
       │                                                   ▼
       │                                          [audit_log + LINE WORKS 返信]
       │
       └─[要承認]─▶ 管理者通知
                       │
                       ├─[承認]─▶ [approved] ─▶ [applied]
                       │
                       ├─[却下]─▶ [rejected] (返信のみ)
                       │
                       └─[本人取消]─▶ [cancelled]
```

### 自動承認ルール (本人が自動反映できる)

- 自分の勤怠 / シフト照会 (DB 書き換えなし)
- **当日中** の打刻漏れ申請
- **当日中** の軽微修正申請 (当日分の時刻変更、±2 時間以内)

### 管理者承認が必要

- 過去日 (昨日以前) の修正
- 締め済み期間の修正 (自動却下)
- 他人の勤怠変更
- シフト確定 / 公開
- カード再紐付け

## データモデル

### ExternalAccount

| フィールド | 型 | 備考 |
|---|---|---|
| id | UUID v7 | |
| employee_id | UUID | |
| provider | enum | `lineworks` |
| external_user_id | String | LINE WORKS User ID |
| external_domain_id | Option\<String\> | |
| is_verified | bool | |
| created_at, updated_at | Zoned | |

### AttendanceRequest

| フィールド | 型 | 備考 |
|---|---|---|
| id | UUID v7 | |
| employee_id | UUID | |
| request_type | enum | `correction` / `missing_in` / `missing_out` / `query_attendance` / `query_shift` |
| requested_payload_json | JSON | コマンドの原文とパース結果 |
| status | enum | `requested` / `auto_approved` / `approved` / `rejected` / `applied` / `cancelled` |
| requested_via | enum | `lineworks` / `ui` |
| requested_at | Zoned | |
| reviewed_by_admin_user_id | Option\<UUID\> | |
| reviewed_at | Option\<Zoned\> | |
| review_note | Option\<String\> | |
| applied_event_id | Option\<UUID\> | 反映された punch_event の UUID |

## 禁止事項

- 受信した自由文を曖昧なまま DB 書き換え
- LINE WORKS メッセージだけで管理者権限を与える
- 承認なしに締め済みデータを書き換える
- LINE WORKS 連携失敗を打刻失敗に連鎖させる

## TDD 対象

- 署名検証 (不正署名を確実に弾く、定数時間比較を使う)
- コマンドパーサ (proptest: 任意の入力で panic しない、不明コマンドは「ヘルプ」誘導)
- 承認ルール判定
- 状態遷移 (不正遷移を弾く)

## 実装確認メモ

- 2026-04-21 時点の公式ソース照合メモは
  [docs/archive/2026-04-21-lineworks-official-source-check.md](../archive/2026-04-21-lineworks-official-source-check.md)
  を参照
- セットアップ / 運用手順は
  [docs/archive/2026-04-21-bitwarden-lineworks-setup-runbook.md](../archive/2026-04-21-bitwarden-lineworks-setup-runbook.md)
  を参照
- `developers.worksmobile.com` の API リファレンス本文を直接確認できていない箇所は、
  実装前に公式サンプルまたは公式リファレンスで再照合すること
