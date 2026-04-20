# Fix Plan

## 目的

このブランチの実装を `AGENTS.md` と `docs/spec/` の確定仕様に合わせる。
特に以下を解消する。

- `cargo test --workspace` が通らない
- Admin / Terminal 認証が未実装
- LINE WORKS secret の扱いが仕様違反
- オフライン再送の `source=local_cached` が保持されない
- 未登録カード検出が `audit_log` に残らない
- Admin 勤怠一覧 API が実データを返さない
- Frontend テストが現状の UI / 実装に追随していない

## 方針

- 1 PR = 1 目的を守るなら、本来は以下を分割する
- ただしこのブランチを救済する目的なら、少なくとも「ビルド修正」「認証」「監査/打刻整合」「テスト追随」は分離して進める
- 実装順は `core` → `server` / `terminal` → `web` → test の順にする

## TODO

- [ ] `terminal` crate のコンパイルエラーを修正する
- [ ] Terminal API を Bearer 認証仕様に合わせる
- [ ] Admin API を Session + Cookie 前提に揃える
- [ ] LINE WORKS secret の unsafe fallback を削除する
- [ ] 未登録カード検出時の audit 記録を追加する
- [ ] オフライン再送で `source=local_cached` を保持する
- [ ] Admin 勤怠一覧 API を仕様に沿う形へ修正する
- [ ] Frontend / Rust の検証を green にする

## 詳細タスク

### 1. `terminal` crate のコンパイル修正

- 対象: `crates/terminal/src/rcs380/transport.rs`
- `raw_exchange` の引数型を `UsbTransport::open()` が返す handle 型に合わせる
- 実機テスト補助コードが test build を壊さないことを優先する
- 必要なら診断用関数を generic 化するか、`rusb::Context` ベースへ寄せる

完了条件:

- `cargo test -p terminal` が少なくともコンパイルを通過する

### 2. Terminal API の Bearer 認証

- 対象: `crates/server/src/terminal.rs`
- 対象: `crates/terminal/src/api_client.rs`
- 対象: terminal 設定の読み出し箇所

実施内容:

- `/api/terminals/me/*` と打刻 API に Bearer 認証を要求する
- token を URL や body ではなく `Authorization: Bearer <token>` で送る
- server 側は token を検証して terminal を識別する
- `docs/spec/07_security.md` にある通り、token は平文ハードコードしない
- 現段階で暗号化保存まで未着手なら、少なくとも「設定読み出し口を secret 前提へ固定し、平文埋め込みを禁止」まで進める

確認観点:

- 認証なしリクエストは 401
- 不正 token は 401
- 正常 token のみ打刻可能

### 3. Admin API の Session + Cookie 対応

- 対象: `crates/server/src/admin.rs`
- 対象: 関連する session 管理コード一式

実施内容:

- Admin API を無認証公開しない
- `admin_session` Cookie を前提に保護する
- cookie 属性を `HttpOnly`, `Secure`, `SameSite=Strict` にする
- 認証方式が未完成なら、少なくとも「無認証で create/update/delete できる状態」を先に止める

確認観点:

- 未ログインで `/api/admin/*` へアクセスすると拒否される
- state-changing request が session 必須になる

### 4. LINE WORKS secret fallback の削除

- 対象: `crates/server/src/main.rs`
- 対象: `crates/server/src/lineworks.rs`

実施内容:

- `LINEWORKS_BOT_SECRET` 未設定時に `"dummy_secret"` を使う実装を削除する
- secret 未設定なら callback router を起動しない、または起動失敗させる
- Bitwarden / 環境変数注入前提から逸脱しないようにする

確認観点:

- secret 未設定で署名検証が成立しない
- 固定既知値で callback を通せない

### 5. 未登録カード検出の監査ログ

- 対象: `crates/core/src/application/attendance.rs`
- 対象: `crates/server/src/infra/sqlite.rs` の audit append 経路

実施内容:

- 未登録カード時に `Notifier::UnregisteredCardDetected` だけでなく `audit_log` へ記録する
- action 名は spec と揃える
- metadata にはカード識別子や発生時刻を必要最小限で持たせる

確認観点:

- 未登録カード問い合わせで audit が 1 件残る
- 通知失敗が打刻系レスポンス失敗に連鎖しない

### 6. オフライン再送の `source=local_cached`

- 対象: `crates/terminal/src/offline.rs`
- 対象: `crates/terminal/src/api_client.rs`
- 対象: `crates/server/src/terminal.rs`

実施内容:

- local cache に `source` を保持する
- オフライン確定時は `source=local_cached` で保存する
- 再送時もその値を server に送る
- server 側で `"terminal"` に上書きしない

確認観点:

- オフライン打刻が DB 上で `local_cached` として保存される
- オンライン打刻は通常 source を維持する

### 7. Admin 勤怠一覧 API の修正

- 対象: `crates/server/src/admin.rs`
- 対象: 必要なら `core` の集計 use case

実施内容:

- `Uuid::nil()` + `now..now` の暫定実装をやめる
- 仕様に沿って employee / 期間を受けてデータを返す API にする
- 最低でも「データが取れないダミー API」をなくす
- 月次勤怠ビューを返すのか、生 punch 一覧を返すのかを仕様に合わせて整理する
- 未確定なら ADR / spec 更新が必要

確認観点:

- 実データが UI に表示される
- 締め日基準の期間計算に将来接続できる構造になっている

### 8. テスト修正と追加

- 対象: `web/admin/src/App.test.tsx`
- 対象: `web/terminal/src/App.test.tsx`
- 対象: 追加が必要な Rust test

実施内容:

- 現在の UI 文言に合わせて壊れた見出しテストを更新する
- `fetch('/api/...')` を使う admin UI test では fetch を mock する
- terminal UI test では `@tauri-apps/api/core` と `@tauri-apps/api/event` を mock する
- 認証、未登録カード監査、オフライン再送 source 保持について Rust 側のテストを足す
- テストの仕様記述は `AGENTS.md` に従い日本語で表現する

確認観点:

- `pnpm -C web/admin test`
- `pnpm -C web/terminal test`
- `cargo test --workspace`

## 推奨実行順

1. `terminal` コンパイル修正
2. LINE WORKS secret fallback 削除
3. Terminal Bearer 認証
4. Admin Session 保護
5. 未登録カード audit
6. オフライン再送 source 修正
7. Admin 勤怠 API 修正
8. テスト更新
9. 全検証実行

## 最終確認コマンド

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
