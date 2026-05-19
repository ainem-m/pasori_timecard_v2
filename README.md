# docs/

このディレクトリはコーディングエージェント用の詳細仕様・判断記録・用語集を
格納する。

## ファイル構成

```
docs/
├── glossary.md               # 用語集 (コード名 / UI 表記 / 説明)
├── spec/                     # 詳細仕様 (章ごと)
│   ├── overview.md           # 全体俯瞰、ユーザー、ユースケース
│   ├── 01_nfc_and_punch.md   # NFC 読取、打刻、確認 UI
│   ├── 02_attendance.md      # 勤怠表、集計、打刻修正
│   ├── 03_shift.md           # シフト管理
│   ├── 04_lineworks.md       # LINE WORKS 連携
│   ├── 05_audit_and_backup.md # 監査・バックアップ・運用
│   ├── 06_data_model.md      # データモデル詳細
│   └── 07_security.md        # 認証・シークレット
├── verification/             # 実機検証・手動 E2E・受け入れ確認手順
│   └── README.md
└── adr/                      # 判断記録 (時系列)
    ├── 0001-tech-stack.md
    ├── 0002-architecture-c-plan.md
    ├── 0003-mvp-scope.md
    ├── 0004-tdd-twada.md
    ├── 0005-lineworks-full-mvp.md
    ├── 0006-project-structure.md
    └── 0007-timezone-asia-tokyo.md
```

## Agent への指示 (AGENTS.md §5 の補足)

### 着手前に読むべき順番

1. `AGENTS.md` (ルート)
2. `docs/glossary.md` (用語確認)
3. `docs/spec/overview.md` (全体像)
4. タスクに関係する `docs/spec/0X_*.md`
5. 関連 ADR (タスクが既存判断に関わる場合)

### 新しい判断をしたくなったら

- **実装に入る前に ADR を 1 件書く**
- ADR 番号は既存最大 + 1
- 状態は `Proposed` → レビュー → `Accepted`
- 既存 ADR を**上書きしない**。新 ADR で supersede する

### spec 変更を伴う PR

- 該当 `docs/spec/*.md` を同一 PR で更新
- 用語を増やしたら `docs/glossary.md` も更新
- 変更理由が大きいなら ADR を追加

## Local Helpers

Bitwarden から secret / token を注入して起動する補助スクリプト:

```bash
scripts/bw-run-server.sh
scripts/bw-run-terminal.sh
```

前提:

- `bw` CLI が利用可能
- 既に `BW_SESSION` を export 済み、または `BW_MASTER_PASSWORD` を設定済み
- Bitwarden に以下の item 名が存在する
  - `lineworks-bot-id`
  - `lineworks-bot-secret`
  - `lineworks-api-token`
  - 必要なら `lineworks-admin-channel-id`
  - `terminal-api-token`

`scripts/bw-run-terminal.sh` は以下を使う:

- `SERVER_API_URL`
  - 未指定時は `http://localhost:8080/api`
- `BW_TERMINAL_TOKEN_ITEM`
  - 未指定時は `terminal-api-token`
