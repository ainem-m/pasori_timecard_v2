# verification/

このディレクトリは、実機検証・手動 E2E・受け入れ確認の**実施手順**を置く。

配置方針は [ADR 0013](../adr/0013-verification-doc-location.md) に従う。

## ここに置くもの

- 手動 E2E チェックリスト
- リリース前受け入れ確認手順
- 実機検証 runbook
- 実施環境、使用機材、前提条件、操作手順、確認方法、証跡取得方法
- 対応する `docs/spec/` 要求への参照

## 現行文書

| 文書 | 用途 | 主な traceability |
|---|---|---|
| [e2e-manual-checklist.md](./e2e-manual-checklist.md) | PaSoRi RC-S380 実機打刻、offline -> reconnect -> sync、Admin、LINE WORKS の半手動確認 | `docs/spec/01_nfc_and_punch.md`, `docs/spec/07_security.md`, `docs/adr/0013-verification-doc-location.md` |

## 自動 E2E との境界

自動 E2E は UI と API の回帰確認、実機 E2E は PaSoRi RC-S380・実カード・実 Terminal・実 Server を通した確認として扱う。
mock reader や自動注入イベントだけの確認結果は、実機 E2E の代替証跡にしない。

macOS ローカルでは公式 `tauri-driver` が desktop WebDriver をサポートしないため、
`pnpm -C web/terminal test:e2e` は Playwright + Vite + mocked Tauri command/event による
Terminal UI 回帰確認として実行する。Tauri 実ウィンドウを使う `tauri-driver` 確認は
`pnpm -C web/terminal test:e2e:tauri` として Linux / Windows の対応環境で実行する。

## ここに置かないもの

- 製品仕様や受け入れ条件
  - `docs/spec/` に置く
- 検証項目の要否や合格基準
  - `docs/spec/` を正本とする
- 一時的なメモや調査途中の叩き台
  - `docs/archive/` または issue に置く
