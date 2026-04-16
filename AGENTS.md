# AGENTS.md

このファイルは Claude Code / Codex などのコーディングエージェント向けの
**短く決定的な** 操作マニュアルである。
詳細仕様は `docs/spec/`、判断理由は `docs/adr/`、用語は `docs/glossary.md`
に分離している。迷ったら ADR を読むか、ADR を 1 件書いてから実装すること。

このファイルに書かれた事項は **確定事項** である。両論併記はない。
選択の余地があるように読める記述を見つけたら、それはバグなので ADR に起票すること。

---

## 1. プロジェクトの目的

- v1 (`pasori_timecard`) の Python 実装を破棄し、**Rust ベースのクロスプラットフォーム勤怠システム**として再構築する
- PaSoRi / FeliCa カードによる打刻、従業員管理、勤怠集計、シフト管理、LINE WORKS 連携をひとつの製品として統合する
- **配布可能な院内サーバー (Rust axum) + 打刻端末 (Tauri Desktop App) + Web 管理画面 (React SPA, Server から配信)** の 3 コンポーネント構成とする

### 非目的 (MVP 時点)

- 給与計算エンジン
- 複数拠点同期
- クラウド SaaS 化
- 顔認証 / 生体認証
- モバイルネイティブアプリ

---

## 2. 技術スタック (確定)

### Rust 側
| 項目 | 確定値 |
|---|---|
| MSRV | **Rust 1.85+ (2024 edition)** |
| 非同期ランタイム | `tokio` |
| HTTP サーバー | `axum` |
| SQLite アクセス | `sqlx` (async, コンパイル時 SQL 検査) |
| マイグレーション | `sqlx migrate` (`migrations/` ディレクトリ) |
| 時刻 | `jiff` (**Asia/Tokyo で timezone-aware 保存**) |
| エラー型 | `thiserror` (core / ライブラリ側) + `anyhow` (app / バイナリ側) |
| ロギング | `tracing` + `tracing-subscriber` + `tracing-appender` (日次ローテ) |
| NFC | `pcsc` crate (PC/SC 経由、PaSoRi RC-S380 等) |
| パスワードハッシュ | `argon2` (Argon2id、OWASP 推奨パラメータ) |
| UUID | `uuid` (**v7 を使用**、時系列ソート可能) |
| Web UI 埋め込み | `rust-embed` (React ビルド成果物を Server バイナリに同梱) |
| 型共有 | `specta` v2 の standalone モード + `tauri-specta` (Tauri 側) |

### Frontend 側
| 項目 | 確定値 |
|---|---|
| Node | **22.x LTS** |
| Package manager | **pnpm** |
| UI フレームワーク | **React** + TypeScript + Vite |
| UI ライブラリ | **shadcn/ui + Tailwind CSS** |

### Desktop shell
| 項目 | 確定値 |
|---|---|
| フレームワーク | **Tauri 2.x** |
| Tauri Command 命名 | Rust 側 snake_case、Tauri 自動変換で TS 側 camelCase |

### テスト / 品質
| 項目 | 確定値 |
|---|---|
| Rust unit / integration | `cargo test` |
| スナップショット | `insta` |
| プロパティベース | `proptest` (domain policy の検証に必須) |
| Frontend unit | `vitest` |
| E2E | `WebDriverIO` + `tauri-driver` |
| Lint (Rust) | `cargo clippy --workspace --all-targets -- -D warnings` |
| Format (Rust) | `rustfmt` 標準設定 (`rustfmt.toml` は置かない) |
| Lint (TS) | `typescript-eslint` strict + `eslint-plugin-react-hooks` |
| Format (TS) | `prettier` 標準設定 |

### CI / 配布
| 項目 | 確定値 |
|---|---|
| CI | **GitHub Actions** (MVP は lint + test のみ、ビルドは手元) |
| 配布形式 | macOS `.dmg` / Windows `.msi` / Linux `.deb` + `.AppImage` |
| コード署名 | **v1.0 リリース前に別途判断** (MVP はスキップ) |

### シークレット管理
| 項目 | 確定値 |
|---|---|
| 保存 | **Bitwarden CLI (`bw get`)** で起動スクリプトが環境変数に展開 |
| 禁止事項 | 設定ファイル / コードに平文シークレットを置かない |

---

## 3. アーキテクチャ概要

### 3 コンポーネント構成

```
┌──────────────────┐      ┌─────────────────────────────────────┐
│ 打刻端末          │      │ 院内サーバー (Raspberry Pi 等)        │
│ (Tauri App)       │──API│ Rust axum + SQLite (正本)            │
│ キオスクモード     │      │ ├─ 打刻 API                          │
│ NFC 読取          │      │ ├─ 管理 API                          │
│ Local cache       │      │ ├─ Web 管理画面配信 (rust-embed SPA) │
└──────────────────┘      │ └─ LINE WORKS Bot callback           │
         ▲                 └─────────────┬───────────────────────┘
         │                               │ Cloudflare Tunnel
         │                               ▼
┌──────────────────┐              ┌────────────────┐
│ 管理クライアント  │◀── HTTPS ───│ LINE WORKS     │
│ (ブラウザ)        │              │ Bot callback   │
└──────────────────┘              └────────────────┘
```

### クレート / ディレクトリ構成

```
/crates
  /core       # domain + application + infra traits (Terminal と Server が共有)
  /terminal   # Tauri 打刻端末 (NFC + HTTP client)
  /server     # axum Server + Web UI 配信 + LINE WORKS callback 受信
  /import_v1  # v1 SQLite を読んで v2 へ取り込む CLI ツール
/web
  /admin      # 管理画面 SPA (React + shadcn/ui + Vite) → Server が配信
  /terminal   # 打刻端末 UI (React + shadcn/ui + Vite) → Tauri 内で動作
/docs
  /spec       # 詳細仕様 (章ごと)
  /adr        # 判断記録
  glossary.md # 用語集
/migrations   # sqlx migrate 用
AGENTS.md
Cargo.toml    # workspace
pnpm-workspace.yaml
```

### 責務分担

| コンポーネント | 責務 |
|---|---|
| `core` | エンティティ、値オブジェクト、業務ルール、use case、repository/notifier/reader trait |
| `terminal` | NFC 読取、打刻 UI (React embedded)、Server API 呼び出し、オフライン時の local cache |
| `server` | データ正本管理、管理 API、Web SPA 配信、LINE WORKS callback、通知送信 |
| `web/admin` | 管理画面 UI (従業員管理、勤怠表、打刻修正、監査ログ、シフト、通知設定) |
| `web/terminal` | 打刻確認 UI (推定種別表示、自動確定カウントダウン、直近履歴) |
| `import_v1` | v1 SQLite の全履歴 (従業員・カード・打刻) を v2 形式に移行する CLI |

### 禁止されるレイヤ跨ぎ

- UI 層 (`web/*` / `terminal` の React) から SQL を直接叩くこと
- UI 層から NFC SDK / LINE WORKS API を直接叩くこと
- `core` が `tauri` / `axum` / `reqwest` / `sqlx` のいずれかに直接依存すること (trait 越しのみ)
- 通知送信を打刻トランザクションの同期必須処理にすること

---

## 4. 検証コマンド (変更後に必ず実行)

**1 PR あたり 1 目的**。以下が全て green でない成果物は提出しない。

### Rust
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

### Frontend (web/admin, web/terminal 両方)
```bash
pnpm -C web/admin lint
pnpm -C web/admin typecheck
pnpm -C web/admin test
pnpm -C web/terminal lint
pnpm -C web/terminal typecheck
pnpm -C web/terminal test
```

### E2E (任意だが機能 PR では推奨)
```bash
pnpm -C web/terminal test:e2e   # WebDriverIO + tauri-driver
```

### 起動確認
```bash
cargo run -p server         # port 8080 で起動
cargo run -p terminal       # Tauri Dev Window
```

---

## 5. Agent の作業ループ

1. 着手前に該当する `docs/spec/*.md` と関連 ADR を読む
2. 要求に未確定事項があれば **先に ADR を 1 件書いてから実装** する
3. TODO リストを書き出す (下記 TDD 規約参照)
4. `core` → `application` → `infra` → `ui` の順でファイルを触る
5. 1 TODO ごとに TDD サイクルを回す (Red → Green → Refactor)
6. 変更後に検証コマンドを全て実行し、green を確認する
7. 同一 PR に実装とテストが両方含まれているか確認する
8. コミットメッセージは **Conventional Commits** に従う (`feat:`, `fix:`, `refactor:`, `test:`, `docs:`, `chore:`)

### 1 PR = 1 目的の原則

- UI と domain を同じ PR で同時に変更しない
- 複数機能を束ねない
- リファクタリングと機能追加を混ぜない

---

## 6. TDD 規約 (t-wada 流)

### 原則

- **Red → Green → Refactor** サイクルを厳守する
- **失敗を示すテストなしにプロダクションコードを書かない**
- テストをパスする**最小のコード**を書く (仮実装を許容)
- Green を保ちながらリファクタリングする。Red 中はリファクタ禁止

### 手順

1. **TODO リストを書き出す** (実装前に何をやるか列挙する)
2. TODO から 1 つ選び、**失敗するテストを書く** (Red を確認)
3. **最小のコード**で通す (仮実装 / 明白な実装どちらでも可、Green を確認)
4. **三角測量** で一般化する (別ケースのテストを追加、ハードコードを解消)
5. リファクタリング (テストは Green を保つ)
6. TODO を 1 つ消す、新しい TODO が見えたら追加
7. 次の TODO へ

### テストコードの書き方

- **テストの仕様記述は日本語で表現する**
  - Rust など識別子ベースのテストは **英語 ASCII `snake_case` の関数名**を使い、**直前の日本語コメント**で仕様を表現する
  - Vitest / Jest など表示名ベースのテストは **日本語の表示名**を使う
  ```rust
  #[test]
  // 最終打刻が前日なら次は出勤と推定する。
  fn decides_clock_in_when_last_event_was_on_previous_day() { ... }
  #[test]
  // 連続スキャン無視時間内の同一カードは重複登録されない。
  fn ignores_duplicate_scan_within_suppression_window() { ... }
  ```
  ```typescript
  it('締め日が15日なら前月16日から当月15日までが当月扱いになる', () => { ... })
  ```
- **Given-When-Then** または **Arrange-Act-Assert** で構造化する
- **DRY より DAMP** (Descriptive And Meaningful Phrases)
  - 重複を恐れずにテストを読みやすく保つ
- 1 テスト 1 アサーションを目指す (関連アサーションが多い場合は可)

### 適用範囲

| 層 | 必須テスト | 備考 |
|---|---|---|
| `core::domain` | **必須 (TDD)** | proptest も積極的に使用 |
| `core::application` | **必須 (TDD)** | use case ごとに integration test |
| `infra_sqlite` | 統合テスト必須 | 実 SQLite に対してテスト |
| `infra_nfc` | mock reader で単体 + 実機で手動 | |
| `server` HTTP handler | 統合テスト必須 | `axum_test` で request/response |
| `terminal` UI | E2E (WebDriverIO) | happy path は必須 |
| `web/admin` | vitest + Playwright (後続) | vitest 必須 |

### Agent への明示指示

- 機能タスクを受けたら **最初に TODO リストを Markdown で提案** する
- TODO を 1 つずつ TDD サイクルで実装する
- **実装コードと同じコミットにテストを含める**。テストなしの実装は差し戻し対象
- テストファースト違反 (先に実装を書いた) は正直に申告する
- 仮実装 → 三角測量 → 一般化のプロセスをコミット単位で残すことを推奨

---

## 7. 主要 trait シグネチャ (確定)

agent は以下の trait 形を毎回新造してはならない。変更する場合は ADR を書くこと。

### 7.1 `ReaderBackend` (crates/core/src/port/reader.rs)

```rust
use jiff::Zoned;

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct CardId(pub String);   // FeliCa IDm の hex 表現

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReaderStatus {
    Disconnected,
    Connecting,
    Ready,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct CardScanned {
    pub card_id: CardId,
    pub scanned_at: Zoned,   // Asia/Tokyo aware
}

#[derive(Debug, thiserror::Error)]
pub enum ReaderError {
    #[error("reader not connected")]
    NotConnected,
    #[error("pcsc error: {0}")]
    Pcsc(String),
    #[error("other: {0}")]
    Other(String),
}

#[async_trait::async_trait]
pub trait ReaderBackend: Send + Sync {
    async fn start(&self) -> Result<(), ReaderError>;
    async fn stop(&self) -> Result<(), ReaderError>;
    fn status(&self) -> ReaderStatus;
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<CardScanned>;
}
```

### 7.2 `PunchPolicy` (crates/core/src/port/policy.rs)

```rust
use jiff::Zoned;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PunchEventType {
    ClockIn,
    ClockOut,
    // 以下は将来拡張、MVP では未使用
    BreakStart,
    BreakEnd,
    TemporaryOut,
    TemporaryReturn,
    ManualCorrection,
}

#[derive(Debug, Clone)]
pub struct PunchEventRef {
    pub event_type: PunchEventType,
    pub occurred_at: Zoned,
}

pub trait PunchPolicy: Send + Sync {
    /// 直近の打刻履歴 (降順) から、次の打刻種別を推定する
    fn decide(&self, recent: &[PunchEventRef], now: &Zoned) -> PunchEventType;
}

pub struct DefaultPunchPolicy;   // MVP 実装 (v1 互換ロジック)
```

### 7.3 `RoundingPolicy` (crates/core/src/port/policy.rs)

```rust
use jiff::Zoned;

pub trait RoundingPolicy: Send + Sync {
    /// 集計時に適用する時刻丸め。MVP の既定は NoRounding (素通し)。
    fn round(&self, event_type: PunchEventType, at: &Zoned) -> Zoned;
}

pub struct NoRounding;   // MVP 既定
```

### 7.4 Repository traits (crates/core/src/port/repo.rs)

**方針: generic CRUD ではなく、業務に必要な操作だけを狭く公開する**。

```rust
use async_trait::async_trait;
use jiff::Zoned;
use uuid::Uuid;

#[async_trait]
pub trait EmployeeRepository: Send + Sync {
    async fn list_active(&self) -> Result<Vec<Employee>, RepoError>;
    async fn find(&self, id: Uuid) -> Result<Option<Employee>, RepoError>;
    async fn find_by_card(&self, card_id: &CardId) -> Result<Option<Employee>, RepoError>;
    async fn create(&self, input: NewEmployee) -> Result<Employee, RepoError>;
    async fn update(&self, id: Uuid, patch: EmployeePatch) -> Result<Employee, RepoError>;
    async fn deactivate(&self, id: Uuid) -> Result<(), RepoError>;
}

#[async_trait]
pub trait CardRepository: Send + Sync {
    async fn find(&self, card_id: &CardId) -> Result<Option<Card>, RepoError>;
    async fn bind(&self, card_id: &CardId, employee_id: Uuid) -> Result<Card, RepoError>;
    async fn unbind(&self, card_id: &CardId) -> Result<(), RepoError>;
}

#[async_trait]
pub trait PunchRepository: Send + Sync {
    async fn insert(&self, event: NewPunchEvent) -> Result<PunchEvent, RepoError>;
    async fn recent_for_employee(
        &self,
        employee_id: Uuid,
        limit: usize,
    ) -> Result<Vec<PunchEvent>, RepoError>;
    async fn list_in_range(
        &self,
        employee_id: Uuid,
        from: &Zoned,
        to: &Zoned,
    ) -> Result<Vec<PunchEvent>, RepoError>;
    async fn update(&self, id: Uuid, patch: PunchPatch, reason: String) -> Result<PunchEvent, RepoError>;
    async fn soft_delete(&self, id: Uuid, reason: String) -> Result<(), RepoError>;
}

#[async_trait]
pub trait ShiftRepository: Send + Sync {
    async fn list_for_month(&self, employee_id: Uuid, year_month: YearMonth) -> Result<Vec<ShiftAssignment>, RepoError>;
    async fn upsert(&self, input: ShiftAssignmentInput) -> Result<ShiftAssignment, RepoError>;
    async fn delete(&self, id: Uuid, reason: String) -> Result<(), RepoError>;
}

#[async_trait]
pub trait AuditLogRepository: Send + Sync {
    async fn append(&self, entry: NewAuditLog) -> Result<(), RepoError>;
    async fn list(&self, filter: AuditLogFilter) -> Result<Vec<AuditLog>, RepoError>;
    // DELETE 系メソッドは存在しない。append-only
}

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("db error: {0}")]
    Db(String),
}
```

### 7.5 `Notifier` trait (crates/core/src/port/notify.rs)

```rust
#[derive(Debug, Clone)]
pub enum NotifyEvent {
    UnregisteredCardDetected { card_id: CardId, at: Zoned },
    MissingPunchSuspected  { employee_id: Uuid, at: Zoned },
    AdminCorrectionApplied { actor: Uuid, target_punch: Uuid },
    DailyClosingResult     { date: jiff::civil::Date, summary: String },
    ShiftPublished         { target_month: YearMonth },
    // 将来拡張
}

#[async_trait]
pub trait Notifier: Send + Sync {
    /// **非同期 fire-and-forget**。この関数のエラーは打刻処理を失敗させてはならない。
    async fn notify(&self, event: NotifyEvent) -> Result<(), NotifyError>;
}
```

---

## 8. 時刻・数値の規約 (確定)

### 時刻

- **保存も表示も Asia/Tokyo** (`jiff::Zoned`、timezone-aware)
- Terminal と Server の時計は **NTP 同期必須**
- NTP 同期許容誤差は **±10 秒**。超過時の Terminal は打刻画面を無効化
- Server の時計を正本とみなす。Terminal のオンライン打刻は Server 側で `server_recorded_at` を別途記録
- Terminal のオフライン打刻は Terminal 時計で `occurred_at` に記録し、再送時に `source = local_cached` フラグを付ける

### 打刻種別の自動推定 (DefaultPunchPolicy)

```
recent が空                                         → ClockIn
recent[0].occurred_at.date() < today               → ClockIn
recent[0].occurred_at.date() == today かつ ClockIn → ClockOut
recent[0].occurred_at.date() == today かつ ClockOut → ClockIn
```

### 数値 (設定可能、以下が既定値)

| 項目 | 既定値 |
|---|---|
| 打刻後自動確定までの猶予 | **30 秒** |
| OK ボタン長押しでスキップ | **1.0 秒** |
| 同一カード連続スキャン無視 | **5 秒** |
| Desktop → Server pull 間隔 | **30 秒** |
| NTP 許容誤差 | **±10 秒** |
| 締め日 | **15 日** (前月 16 日〜当月 15 日) |
| 日次バックアップ保持 | **30 日** |
| Admin セッション有効期限 | **24 時間** (活動あり時は自動延長) |
| 打刻時刻の保存粒度 | **1 分単位** (丸めは RoundingPolicy、既定は NoRounding) |

### 丸めとポリシー差し替え

- MVP 実装は `NoRounding` (素通し)
- 将来、出勤は 5 分切り上げ・退勤は 1 分単位などを追加する場合は `RoundingPolicy` 実装を差し替える
- `RoundingPolicy` 実装を複数作るのは MVP スコープ外。MVP では NoRounding のみ

---

## 9. データモデル規約

### 全エンティティ共通

- **主キーは UUID v7** (時系列ソート可能、将来の同期に有利)
- `created_at: Zoned`, `updated_at: Zoned` を必ず持つ
- 物理削除はしない。`is_active: bool` または `deleted_at: Option<Zoned>` によるソフトデリート
- 重要更新は監査ログ `audit_log` テーブルに append する

### 最低限必要なテーブル (詳細は `docs/spec/06_data_model.md`)

| テーブル | 概要 |
|---|---|
| `employee` | 従業員 |
| `card` | IC カード ↔ 従業員 の紐付け |
| `punch_event` | 打刻 (Terminal 生成の UUID v7 を punch_id とし、UNIQUE 制約で冪等性保証) |
| `shift_type` | シフト種別マスタ |
| `shift_assignment` | シフト予定 |
| `admin_user` | 管理者 (Argon2id ハッシュ、username + password_hash) |
| `admin_session` | Admin Web セッション |
| `external_account` | LINE WORKS 送信者 ID ↔ 従業員 の紐付け |
| `attendance_request` | LINE WORKS 経由の修正申請・照会履歴 |
| `audit_log` | append-only、DELETE 禁止 |
| `terminal` | 登録済み打刻端末 (API token 発行用) |

### 冪等性保証

- Terminal は打刻ごとに UUID v7 を生成して `punch_id` とする
- Server は `punch_event.id` に UNIQUE 制約を設定
- Terminal オフライン時の溜め込みも、再送時に UNIQUE で重複が弾かれる

---

## 10. 認証と通信

### Terminal ↔ Server

- **API Token (Bearer)** 方式
- Token は Server が Admin Web から発行、Terminal 設定ファイルに記録
- Token はローテ可能、リボケーション可能 (admin 画面から)

### Admin ↔ Server (Web UI)

- **Session + Cookie (HttpOnly, Secure, SameSite=Strict)**
- Session テーブルに保存、24 時間有効、活動で自動延長
- パスワードは Argon2id

### LINE WORKS Bot callback

- `X-WORKS-Signature` による HMAC 署名検証を必須とする (未検証リクエストは 401)
- Bot token は Bitwarden から起動時に環境変数注入

---

## 11. Definition of Done (PR 単位)

以下を**すべて**満たす PR のみ merge 可能。

- [ ] 新規コード行に対応する **テストが同一 PR 内に存在** する
- [ ] `cargo fmt --check` が green
- [ ] `cargo clippy -D warnings` が green
- [ ] `cargo test --workspace` が全て green
- [ ] 関連フロント (`web/*`) の `lint` / `typecheck` / `test` が green
- [ ] テストの**仕様記述が日本語で読める**
- [ ] 監査対象の操作を追加した場合は、現行設定方針に従って `audit_log` 連携を実装
- [ ] 時刻を扱うコードは `jiff::Zoned` (Asia/Tokyo) を使用 (naive date/time 禁止)
- [ ] 新 trait / 新エンティティ導入時は `docs/spec/` または `docs/adr/` を同時更新
- [ ] 仕様変更を伴う PR は `docs/spec/` を更新
- [ ] コミットメッセージが Conventional Commits 形式

---

## 12. 禁止事項 (NO-GO)

Agent がこれらを行った PR は **自動的に差し戻し対象**。

- v1 Python コードの逐語的な Rust 翻訳
- UI 層から SQL / NFC SDK / LINE WORKS API を直接呼ぶ
- `core` crate が `tauri` / `axum` / `sqlx` / `reqwest` / `pcsc` に直接依存する
- `unwrap()` / `expect()` / `panic!()` を production path (非テストコード) に残す
- 時刻を naive に保存する (`chrono::NaiveDateTime` / `DateTime<Utc>` の直接使用)
- 打刻処理を通知送信の成否と同期結合する (通知失敗が打刻失敗を引き起こす設計)
- 既定で有効な監査対象イベントを `audit_log` に残さない
- 平文シークレットを設定ファイル / ソースコード / コミット履歴に残す
- 将来拡張を理由に現時点で不要な抽象を導入する (generic CRUD repository、未使用の event bus 等)
- テストなしで実装を追加する (TDD 違反)
- `#[ignore]` でテストを黙殺する (必要なら ADR を書く)
- 1 PR に複数目的を混ぜる
- `AGENTS.md` / `docs/spec` / `docs/adr` に書かれた確定事項を勝手に変更する (ADR を書いて変更提案する)

---

## 13. 参照

- `docs/spec/` — 詳細仕様
- `docs/adr/` — 判断記録 (時系列で追記)
- `docs/glossary.md` — 用語集 (コード名 / UI 表記 / 説明)

特に重要な ADR:

- `docs/adr/0001-tech-stack.md` — 技術スタック確定の経緯
- `docs/adr/0002-architecture-c-plan.md` — C プラン (Server 中心) 採用
- `docs/adr/0003-mvp-scope.md` — MVP スコープの定義
- `docs/adr/0004-tdd-twada.md` — TDD 規約採用
- `docs/adr/0005-lineworks-full-mvp.md` — LINE WORKS 双方向を MVP に含める判断
- `docs/adr/0009-test-name-language-policy.md` — テスト識別子と言語ポリシー
