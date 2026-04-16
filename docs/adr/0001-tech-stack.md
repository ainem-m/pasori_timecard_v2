# ADR 0001: 技術スタック確定

- **日付**: 2026-04-16
- **状態**: Accepted
- **関連**: AGENTS.md §2

## 背景

v1 は Python + PySide6 + SQLAlchemy。v2 では配布可能なクロスプラットフォーム
製品として作り直す必要があり、言語・フレームワークを一から選定した。

## 決定

| カテゴリ | 採用 | 理由 |
|---|---|---|
| 主言語 | **Rust 1.85+ (2024 edition)** | 単一バイナリ配布、メモリ安全、NFC / SQLite / HTTP を同一言語で扱える |
| Desktop shell | **Tauri 2.x** | Rust バックエンド + Web UI、配布容易、mobile 将来対応可 |
| Frontend | **React + TS + Vite** | エコシステム最大、AI エージェントが最も書きやすい、shadcn/ui が使える |
| UI ライブラリ | **shadcn/ui + Tailwind** | コピペ式で柔軟、Agent が書きやすい |
| Package manager | **pnpm** | Tauri 公式推奨、ディスク効率 |
| Node | **22.x LTS** | 現行 LTS |
| HTTP サーバー | **axum** | tokio ネイティブ、Rust エコシステムで事実上標準 |
| SQLite | **sqlx** | async、compile-time SQL 検査、migration 同梱 |
| Time | **jiff** | timezone-aware がデフォルト、tzdata 内蔵、Asia/Tokyo 運用と相性最良 |
| Error | **thiserror (core) + anyhow (app)** | 標準的パターン |
| Logging | **tracing + tracing-subscriber + tracing-appender** | 非同期対応、構造化ログ、日次ローテ |
| NFC | **pcsc crate** | PC/SC 経由、Win/macOS/Linux 全対応、PaSoRi 実績あり |
| Password hash | **Argon2id** | OWASP 推奨、RFC 9106 標準化済 |
| UUID | **v7** | 時系列ソート可、将来の同期で有利 |
| 型共有 | **specta v2 standalone + tauri-specta** | Tauri command も Server API も同じ仕組みで生成 |
| Test | **cargo test + insta + proptest + vitest + WebDriverIO** | プロパティベースで打刻ポリシーを分厚く検証 |
| 配布 | **.dmg / .msi / .deb + .AppImage** | Tauri 標準 |
| Config | **TOML + directories crate** | Rust 標準的、OS 標準の設定ディレクトリを使用 |
| Secrets | **Bitwarden CLI (`bw get`)** で環境変数注入 | AI agent から実シークレットを守る |

## 代替案と却下理由

- **egui / iced / slint (Rust native GUI)**: エコシステムが Web より弱く、shadcn/ui のような高品質 UI ライブラリがない。Agent 向きでない。
- **rusqlite (同期)**: Tauri + tokio の async フローから浮く。sqlx の migration 同梱の利便性が勝る。
- **chrono / time**: timezone 扱いが jiff より弱い。特に aware 保存方針と合わない。
- **Sentry / GlitchTip**: 医療機関のプライバシー配慮でクラウドエラートラッキングは採用しない。MVP はローカルログのみ。

## 結論

上記で確定。変更には新しい ADR を書くこと。
