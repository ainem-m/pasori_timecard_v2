# 詳細仕様: NFC 読取と打刻

## スコープ

このドキュメントは以下を定義する。

- NFC Reader の抽象と実装
- 打刻フロー (スキャン → 確認 → 確定)
- 打刻種別の自動推定ロジック
- 確認 UI の仕様
- オフライン時の動作

AGENTS.md §7 (主要 trait) と §8 (時刻・数値規約) も参照。

## Reader 抽象

`crates/core/src/port/reader.rs` で定義する。AGENTS.md §7.1 の `ReaderBackend` trait を
使用する。実装は以下:

- **本番**: `infra_nfc::PcscReader` (pcsc crate を使用)
- **テスト/開発**: `infra_nfc::MockReader` (tokio channel で任意のイベントを注入可能)

### iPhone エクスプレスカード対策

v1 で実装されていた「待受対象の切替」挙動は、Reader 実装内部の責務として
閉じ込める。上位レイヤは知らなくてよい。詳細は pcsc / PaSoRi の `RCS380`
のコマンド仕様を参照して実装する。

## 打刻フロー

### オンライン時

```
[Terminal UI]
   ▼ カードスキャン (CardId 取得)
[Terminal] ─ GET /api/terminals/me/card_scanned?card_id=... ─▶ [Server]
                                                                │
                                                                ├─ 従業員解決
                                                                ├─ 直近打刻取得
                                                                └─ PunchPolicy で種別推定
[Terminal] ◀── 200 { employee, recent_events, suggested_type } ──
   ▼ 確認 UI 表示 (氏名・時刻・推定種別)
   ▼ 30 秒カウントダウン / OK 1 秒長押しでスキップ / 種別変更可
[Terminal] ─ POST /api/punches (punch_id=UUID v7, type, occurred_at, client_recorded_at) ─▶ [Server]
                                                                                             │
                                                                                             ├─ 冪等性チェック (UNIQUE)
                                                                                             ├─ 保存
                                                                                             ├─ audit_log (必要時)
                                                                                             └─ Notifier 非同期発火
[Terminal] ◀── 201 { punch_event } ──
   ▼ 「打刻完了」表示 (3 秒後にキオスク画面へ)
```

### オフライン時 (Server 停止 / ネットワーク遮断)

```
[Terminal UI]
   ▼ カードスキャン
[Terminal] ─── Server 疎通確認 (HEAD /health) ─▶ 失敗
   ▼ local cache モードへ切替表示
   ▼ Terminal 側 SQLite に pending_sync で保存 (punch_id, employee_id, card_id, occurred_at)
   ▼ 確認 UI は従業員名・推定種別・直近履歴を local cache から表示
   ▼ 確定後、pending 状態でキオスク画面に戻る

[Terminal] (30 秒間隔で) ─ HEAD /health ─▶ [Server] ◀── 200
   ▼ 再接続検知
   ▼ pending_sync 打刻を古い順に POST /api/punches (client_recorded_at を含む)
   ▼ Server は UNIQUE 制約で重複を自然に弾き、既存なら 409、新規なら 201
   ▼ Terminal 側は successfully_synced に更新
```

## 打刻種別の自動推定

### DefaultPunchPolicy (MVP 既定)

```rust
fn decide(recent: &[PunchEventRef], now: &Zoned) -> PunchEventType {
    let today = now.date();
    match recent.first() {
        None => PunchEventType::ClockIn,
        Some(last) if last.occurred_at.date() < today => PunchEventType::ClockIn,
        Some(last) if last.event_type == PunchEventType::ClockIn => PunchEventType::ClockOut,
        Some(_) => PunchEventType::ClockIn,
    }
}
```

### proptest で検証すべき性質

- 任意の `recent` に対し、戻り値は必ず `ClockIn` または `ClockOut`
- `recent` が空なら必ず `ClockIn`
- `last.occurred_at < today` なら必ず `ClockIn`
- 同日かつ last が `ClockIn` なら必ず `ClockOut`
- 同日かつ last が `ClockOut` なら必ず `ClockIn`

## 確認 UI の仕様

### 表示要素

- 従業員氏名 (大きく、フォント 36px+)
- 打刻時刻 (HH:MM、大きく)
- 推定種別 (出勤 / 退勤、色分け)
- 自動確定カウントダウン (30 → 0 秒)
- 種別変更ボタン (出勤 ↔ 退勤 切替)
- キャンセルボタン
- OK ボタン (長押し 1 秒で即時確定)
- 直近 5 件の打刻履歴 (日付 + 時刻 + 種別)

### 動作

- 30 秒カウントダウン中、どのボタンも押されなければ推定種別で自動確定
- OK ボタン長押し 1 秒 → カウントダウンをスキップして即時確定
- 種別変更ボタンタップ → カウントダウンリセット、種別を切替
- キャンセルボタン → 打刻せずにキオスク画面へ戻る

### アクセシビリティ

- 本文フォントは最小 18px
- 主要ボタンは最小 44×44pt 以上
- コントラスト比は WCAG AA (4.5:1) 以上

## 連続スキャン無視

同一 `CardId` が前回スキャンから設定時間内 (既定 5 秒) に再スキャンされた
場合、Terminal 側で無視する (Server には送信しない)。

- 実装は Terminal 内の in-memory LRU で直近 N 件の (`card_identifier`, last_scanned_at) を保持
- 設定変更は Admin Web から行い、Terminal は起動時と定期更新時に設定を取得

## NTP 同期チェック

- Terminal 起動時、および 10 分ごとに OS の NTP 同期状態を確認
- プラットフォーム別:
  - Linux: `timedatectl status` の `System clock synchronized: yes`
  - macOS: `sntp -s time.apple.com` の offset
  - Windows: `w32tm /query /status` の `Leap Indicator`
- Server との時刻差分も `GET /api/health` のレスポンスヘッダ `Server-Time` で検証
- ±10 秒を超えたら打刻画面を無効化し、「時刻同期エラー」画面を表示

## 未登録カードの取扱

1. Terminal が Server に問い合わせ → Server は `{ status: "unregistered", card_identifier }` を返す
2. Server は audit_log に `event = "unregistered_card_detected"` で記録
3. Server は `Notifier::UnregisteredCardDetected` を非同期発火
4. Terminal は Server から有効従業員一覧を取得する
5. Terminal は従業員氏名のみを一覧表示する (MVP 想定 20 人程度のため検索なし)
6. 操作者が従業員を選択する
7. Terminal は「このカードを 山田太郎 に登録します」の確認画面を表示する
8. 確認後、Terminal API token で Server に紐付けを要求する
9. Server は `card.bind` を audit_log に記録する (`actor_type = "terminal"`)
10. Terminal は local card cache に保存する
11. Terminal は「山田太郎に登録しました」と表示し、打刻せず待受に戻る
12. 直近履歴には「カード登録」を表示する

### Terminal 紐付けの制約

- 操作者認証は挟まない
- オンライン時のみ実行できる
- Server 不通時は「しばらくしてもう一度試してください」と表示する
- 紐付け失敗時は「もう一度試してください」と表示する
- カード ID は Terminal 画面に表示しない
- 有効従業員のみ選択できる
- 既に紐付いたカードは通常打刻し、Terminal から付け替えしない
- 登録後にそのまま打刻はしない
- 次回スキャンから打刻可能にする
- 1 従業員に複数カードを紐付けできる
- 1 カードは同時に 1 従業員だけに紐付く
- Admin Web のカード紐付け機能は事前登録・誤登録修正用に残す

## TDD 対象

AGENTS.md §6 に従い、以下は **必ず** TDD で実装する。

- `DefaultPunchPolicy::decide` (proptest 併用)
- 連続スキャン無視ロジック
- オフライン → オンライン再送の冪等性
- NTP ずれ判定
- Terminal 側の未登録カード紐付けフロー
