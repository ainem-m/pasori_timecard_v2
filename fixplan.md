# Fix Plan

## 目的

2026-04-25 のレビューで見つかった仕様逸脱と実害のある未完成箇所を、`AGENTS.md` / `docs/spec/` / `docs/adr/` に合わせて直す。

この計画は 1 PR = 1 目的を守るための分割案である。実装時は各項目ごとに TODO を細分化し、Red -> Green -> Refactor の順で進める。

## 現在の検証状況

レビュー時点で以下は green。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm -C web/admin lint
pnpm -C web/admin typecheck
pnpm -C web/admin test
pnpm -C web/terminal lint
pnpm -C web/terminal typecheck
pnpm -C web/terminal test
```

ただし green でも仕様未達が残っている。特に Terminal 打刻時刻、オフライン打刻、LINE WORKS 自動承認、監査ログ、session token は優先して直す。

## 優先順位

### P0: 打刻データの正しさ

- Terminal UI が `new Date().toISOString()` で `occurred_at` を送っているため、Asia/Tokyo 保存規約に反する
- Terminal UI が `crypto.randomUUID()` を使っており、punch_id の UUID v7 規約に反する
- オフライン時はカード解決に失敗した時点で UI が止まり、local cache 確定フローに到達しない
- オンライン打刻でも `source: 'terminal'` を送っており、仕様上の `nfc` / `local_cached` と揃っていない

### P1: 承認・監査・認証の正しさ

- ~~LINE WORKS の `出勤忘れ` / `退勤忘れ` が「反映しました」と返信するが、punch_event を作成せず `applied` にも進めない~~ (PR 3 で完了)
- ~~Admin 承認 API は Correction 以外を 409 にしており、MissingIn / MissingOut の手動承認ができない~~ (PR 3 で完了)
- ~~`audit_log` に UPDATE / DELETE 禁止トリガーがない~~ (PR 4 で完了)
- ~~重要操作の監査ログ append 失敗を握りつぶしている箇所がある~~ (PR 4 で完了: `let _ =` → `if let Err(e) = ... { tracing::error! }`)
- ~~従業員 create/update/deactivate が監査ログに残らない~~ (PR 4 で完了)
- Admin session token が random 256bit ではなく UUID v7
- ~~LINE WORKS 署名失敗時に `audit_log` が残らない~~ (PR 4 で完了)
- ~~punch.create_manual (admin 承認による MissingIn/MissingOut 打刻作成) の監査ログがない~~ (PR 4 で完了)
- ~~request.auto_approved (LINE WORKS 自動承認) の監査ログがない~~ (PR 4 で完了)

### P2: 運用仕様と UI 完成度

- Terminal の NTP 同期チェックが OS NTP 状態を確認していない
- Terminal UI は同期チェック失敗をブロックしない場合がある
- Admin Web は主要表示が英語混在で、勤怠製品としての画面完成度が低い
- Admin Web の従業員追加、カード紐付け、打刻修正、申請承認 UI が未完成
- Terminal / Admin とも E2E が未整備

## PR 分割案

## PR 1: Terminal 打刻時刻と UUID v7 を正す ✅

完了。主な変更:

- `crates/terminal/src/punch.rs` 新規: `create_punch_request()` / `create_offline_punch_request()` で UUID v7, Asia/Tokyo, 1分丸め, source="nfc"/"local_cached" を生成
- `crates/terminal/src/main.rs`: `submit_punch` は card_id + event_type のみ受信、punch_id/occurred_at/source は Rust 側で決定
- `web/terminal/src/App.tsx`: `crypto.randomUUID()` / `new Date().toISOString()` を削除、Rust command に委譲
- テスト: Rust unit (UUID v7, timezone, truncation, source), Vitest (invoke params に punch_id/occurred_at/source なし)

## PR 2: Terminal オフライン打刻を実動作にする ✅

完了。主な変更:

- `crates/terminal/src/offline.rs`: `local_card_cache` テーブル追加、`cache_card()` / `find_cached_card()` 実装、`get_unsynced_punches()` が古い順 (`ORDER BY occurred_at ASC`) を返す
- `crates/terminal/src/main.rs`: `resolve_card` が server → local cache フォールバック、background sync が offline request factory を使用
- テスト: cache lookup, unknown card → None, overwrite, chronological order, synced exclusion

## PR 3: LINE WORKS MissingIn / MissingOut を反映可能にする ✅

完了。主な変更:

- `crates/core/src/application/lineworks.rs`: MissingIn/MissingOut の自動承認時に `punch_event` 作成、`applied` ステータスに遷移、`applied_event_id` 保存。`CardRepository::find_by_employee()` 追加
- `crates/server/src/admin.rs`: `approve_attendance_request` で MissingIn/MissingOut をハンドリング (`create_missing_punch()`)、Correction 以外を 409 にしない
- テスト: MissingIn → ClockIn punch_event 作成, MissingOut → ClockOut punch_event 作成

## PR 4: 監査ログを保証する ✅

完了。主な変更:

- `migrations/20260425000000_audit_log_immutable_triggers.sql`: UPDATE/DELETE 禁止トリガー追加
- `crates/server/src/infra/sqlite.rs`: integration test で更新/削除が ABORT になることを検証
- `crates/server/src/admin.rs`: 従業員 create/update/deactivate の監査ログ、MissingIn/MissingOut 承認時の `punch.create_manual` 監査ログ、全 `let _ = repo.append(...)` を `if let Err(e) = ... { tracing::error! }` に変更 (12箇所)
- `crates/server/src/lineworks.rs`: `LineworksAppState` に `audit_repo` 追加、署名なし/不一致時に `lineworks.signature_missing` / `lineworks.signature_invalid` 監査ログ記録、`process_event()` の戻り値を `ProcessEventOutcome` に変更、`AutoApproved` の場合に `request.auto_approved` 監査ログ記録
- `crates/core/src/application/lineworks.rs`: `ProcessEventOutcome` enum 追加 (`AutoApproved`, `RequestCreated`, `HelpSent`, 等)
- `crates/server/src/main.rs`: `lineworks::router()` に `audit_repo` を渡すよう変更
- テスト: 2件の LINE WORKS 署名失敗監査テスト (`audit_logs_missing_signature`, `audit_logs_invalid_signature`)、SQLite トリガーテスト 2件

### PR 4 で未対応 (API endpoint 未実装のためスコープ外)

以下は監査アクション名が仕様に定義されているが、対応する API endpoint が未実装のため監査ログも未実装:

- `card.bind` / `card.unbind` / `card.rebind` — カード紐付け API 未実装
- `punch.soft_delete` — 打刻削除 API 未実装
- `admin.password_change` — パスワード変更 API 未実装
- `settings.update` — 設定永続化機能未実装

## PR 5: Admin session token を random 256bit にする ✅

完了。主な変更:

- `crates/server/src/infra/sqlite.rs`: session id 生成を UUID v7 から `OsRng` 32 bytes の lowercase hex に変更
- `crates/server/src/admin.rs`: cookie builder が string token を受けるよう変更
- `migrations/20260428000000_admin_session_random_token.sql`: `admin_session` を `issued_at` ベースの schema に移行し、既存 session は `created_at` を `issued_at` として引き継ぐ
- テスト: Login cookie が 64 hex chars かつ UUID 形式でないこと、UUID ではない token の session 認証、random token session の logout 削除

## PR 6: Terminal NTP 同期チェックを仕様に寄せる ✅

完了。主な変更:

- `crates/terminal/src/clock.rs` 新規: OS 別 NTP checker と parser を追加 (Linux `timedatectl`, macOS `sntp`, Windows `w32tm`)
- `crates/terminal/src/main.rs`: `check_clock_sync` が OS 同期状態と Server-Time 差分の両方を検証
- `web/terminal/src/App.tsx`: `check_clock_sync` を起動時 + 10 分ごとに実行し、失敗時も時刻同期エラー画面で打刻を無効化
- テスト: OS checker parser、Server-Time 10 秒閾値、clockError 中の `card-scanned` 無視と `submit_punch` 抑止

## PR 7: Admin Web の運用 UI を最低限完成させる ✅

完了。主な変更:

- `web/admin/src/App.tsx`: 管理画面を日本語化し、ダッシュボード / 従業員 / 勤怠 / 修正申請 / 監査ログの導線に整理
- 従業員追加、従業員無効化、カード紐付け、修正申請の承認/却下 UI を追加
- Placeholder の Search / Settings / Bell / Menu を削除
- Audit Logs は `target_id` を含む対象表示に修正
- `crates/server/src/admin.rs`: `POST /api/admin/cards/bind` / `POST /api/admin/cards/unbind` を追加し、`card.bind` / `card.rebind` / `card.unbind` 監査ログを記録
- テスト: 従業員一覧表示、従業員追加 submit、修正申請 approve/reject、audit row の `target_id` 表示、カード bind/unbind 監査

## PR 8: E2E と手動検証を整備する ✅

完了。主な変更:

- `web/terminal`: macOS でも動く `pnpm -C web/terminal test:e2e` を Playwright + Vite + mocked Tauri command/event として追加
- `web/terminal`: Linux / Windows 対応環境向けに `pnpm -C web/terminal test:e2e:tauri` (WebDriverIO + tauri-driver) を分離
- Terminal happy path (card scan -> confirm -> punch submitted) を自動 E2E 化
- `docs/verification/e2e-manual-checklist.md`: offline -> reconnect -> sync の半手動手順、PaSoRi RC-S380 実機検証、証跡、Pass / Fail / Blocked 判定を具体化
- `docs/verification/README.md`: 自動 E2E と実機 E2E の境界、macOS での tauri-driver 非対応を明記

## 横断ルール

- 仕様未確定なら実装前に ADR を追加する
- production path に `unwrap()` / `expect()` / `panic!()` を残さない
- UI 層から SQL / NFC SDK / LINE WORKS API を直接呼ばない
- `core` に `axum` / `tauri` / `sqlx` / `reqwest` / `pcsc` を入れない
- 時刻は保存も表示も Asia/Tokyo aware にする
- テストの仕様記述は日本語にする

## 最終確認コマンド

各 PR の最後に最低限以下を実行する。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm -C web/admin lint
pnpm -C web/admin typecheck
pnpm -C web/admin test
pnpm -C web/terminal lint
pnpm -C web/terminal typecheck
pnpm -C web/terminal test
```

Terminal 機能 PR では可能なら以下も実行する。

```bash
pnpm -C web/terminal test:e2e
```
