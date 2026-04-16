# ADR 0006: プロジェクト構造 (クレート構成とディレクトリ)

- **日付**: 2026-04-16
- **状態**: Accepted
- **関連**: ADR 0002 (C プラン), AGENTS.md §3

## 背景

ADR 0002 で C プラン (Server 中心) を採用したことにより、当初想定の 2 crates
(core + app) では足りなくなった。Terminal と Server という 2 つのデプロイ
対象が生まれ、両者が `core` を共有する必要がある。

一方、過剰な分割は AGENTS.md 禁止事項「将来拡張を理由に現時点で過剰抽象化する」
に抵触する。MVP に必要な最小限の分割に留める。

## 決定

### クレート構成 (MVP)

```
/crates
  /core       # domain + application + infra traits (deps: jiff, uuid, thiserror, async_trait 等)
  /terminal   # Tauri 打刻端末 (deps: core, tauri, pcsc, reqwest, sqlx)
  /server     # axum Server (deps: core, axum, sqlx, tokio, rust-embed)
  /import_v1  # v1 SQLite → v2 インポート CLI (deps: core, sqlx, clap)
```

### Workspace Cargo.toml

```toml
[workspace]
members = ["crates/core", "crates/terminal", "crates/server", "crates/import_v1"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.85"

[workspace.dependencies]
jiff = "0.1"
uuid = { version = "1", features = ["v7"] }
thiserror = "1"
anyhow = "1"
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
sqlx = { version = "0.8", features = ["runtime-tokio", "sqlite", "macros"] }
tracing = "0.1"
tracing-subscriber = "0.3"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
specta = { version = "2", features = ["derive"] }
argon2 = "0.5"
```

### Frontend 構成

```
/web
  /admin      # React SPA (pnpm workspace)
  /terminal   # React SPA (pnpm workspace, Tauri 内で動作)
pnpm-workspace.yaml
```

### pnpm-workspace.yaml

```yaml
packages:
  - 'web/*'
```

### 依存ルール

- `core` は他のどの crate にも依存しない (純粋なドメイン)
- `terminal` と `server` は `core` に依存する
- `terminal` と `server` は互いに依存しない
- `import_v1` は `core` に依存する (domain type を再利用)
- `core` は `tauri` / `axum` / `reqwest` / `sqlx` に**直接依存しない** (trait 越しのみ)

### `core` の内部構造

```
crates/core/src/
  lib.rs
  domain/
    mod.rs
    employee.rs       # Employee, NewEmployee, EmployeePatch
    card.rs           # Card, CardId
    punch.rs          # PunchEvent, PunchEventType, PunchEventRef
    shift.rs          # ShiftAssignment, ShiftType
    audit.rs          # AuditLog, NewAuditLog
    request.rs        # AttendanceRequest, ExternalAccount
    admin.rs          # AdminUser
    time.rs           # 時刻関連ユーティリティ
  application/
    mod.rs
    record_punch.rs   # use case: 打刻を記録する
    manage_employee.rs
    manage_card.rs
    compute_timesheet.rs
    approve_request.rs
    (others)
  port/                # infra 層が実装すべき traits
    mod.rs
    reader.rs         # ReaderBackend
    repo.rs           # EmployeeRepository, CardRepository, ...
    notify.rs         # Notifier
    policy.rs         # PunchPolicy, RoundingPolicy
  error.rs
```

## 分割トリガ (post-MVP)

以下の条件が発生したら該当層を別 crate に切り出す。

| トリガ | 切り出す先 |
|---|---|
| SQLite 以外のストレージを追加する | `infra_sqlite` を別 crate に |
| NFC のモックと実装を実行時切替したい | `infra_nfc` / `infra_nfc_mock` に分離 |
| LINE WORKS 以外の通知チャネル追加 | `infra_notify` / `infra_notify_lineworks` に分離 |
| core のコンパイル時間が 5 分超 | 機能別に domain / application を分離 |

## 代替案と却下理由

- **2 crate (core + app)**: Terminal と Server を 1 バイナリにする前提だったが、Tauri と axum を同居させると起動モード切替ロジックが複雑化。2 バイナリに分けた方がシンプル。
- **6 crate (当初案)**: infra 層を最初から分離する案。MVP の範囲では infra が 1 種類ずつなので意味が薄い。
- **monorepo + 単一 Rust crate**: terminal と server を feature flag で切り替え。ビルド時間とコード複雑度が悪化する。

## 結論

4 crate 構成で MVP を開始する。分割トリガに該当したら ADR を追加して分離する。
