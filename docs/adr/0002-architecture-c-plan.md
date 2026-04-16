# ADR 0002: アーキテクチャ C プラン (Server 中心) 採用

- **日付**: 2026-04-16
- **状態**: Accepted
- **関連**: AGENTS.md §3

## 背景

当初は「配布単位は原則 1 アプリ / ローカル完結」の方針で、Desktop アプリ単独
構成 (A プラン) を検討していた。しかし以下の要求が積み重なり、構成の見直しが
必要になった。

1. LINE WORKS の双方向連携 (照会 + 修正申請) を MVP に含めたい
   → 公開 HTTPS の callback 受信口が必須
2. 「管理は別マシンからもしたい」「家からも勤怠を見たい」という運用要求
   → 複数端末アクセスが実質的に必要
3. 打刻端末はキオスク固定にしたい
   → 管理機能を Desktop に載せる必要がない

A プラン (Desktop 単独) ではこれらを満たせない。B プラン (MVP 時点で placeholder
server だけ作る) は実益が薄い。結果として C プラン全面採用を決定した。

## 決定

**3 コンポーネント構成**

- **Server** (院内 Raspberry Pi / Mac mini 等 + Cloudflare Tunnel)
  - Rust `axum`, `sqlx(SQLite)`, `rust-embed` で SPA 同梱の単体バイナリ
  - データ正本を保持
  - LINE WORKS callback 受信、管理 API、Web 管理画面配信
- **Desktop Terminal** (Tauri 2.x)
  - NFC 読取と打刻確認 UI のみ。キオスクモード固定
  - Server に打刻を REST POST
  - **Server 停止時のフォールバック**: Terminal の SQLite に一時保存、復旧時に再送信
- **Admin Client** (ブラウザ)
  - Web UI で Server にアクセス
  - 従業員管理、勤怠表、打刻修正、監査ログ、シフト、通知設定を担当

Core crate は Terminal / Server / import_v1 で共有する。

## 失うもの / 得るもの

### 得るもの
- LINE WORKS 双方向連携が成立する
- 複数打刻端末 / 複数管理者クライアントが自然に対応可能
- 院外からの管理アクセスが可能 (Cloudflare Tunnel 経由)
- 将来の複数拠点対応の素地

### 失うもの
- 「Desktop 単独で完結」の原則 (AGENTS.md 最重要方針 3 を書き換え)
- 配布先ごとに **Server 1 台 + Cloudflare Tunnel 契約 + Bitwarden アカウント + LINE WORKS Bot** の運用責任が発生
- MVP 実装工数の増加

## 運用上の前提 (配布先に要求するもの)

1. 院内に常時稼働する小型サーバー (Raspberry Pi 4 以上 / Mac mini / 余剰 PC)
2. Cloudflare アカウント (無料枠) + Tunnel 設定
3. Bitwarden アカウント (シークレット管理)
4. LINE WORKS Bot (admin が作成、client secret を Bitwarden に保管)

これらは運用ドキュメントに明記する。

## 代替案と却下理由

- **A プラン (Desktop 単独)**: LINE WORKS 受信を諦めることになる。運用要求を満たせない。
- **A+ プラン (MVP 送信のみ、v1.1 で Server 追加)**: 「v1.1 に回した機能は実際には回らない」リスク。修正申請は最初から欲しい。
- **C プラン for VPS (外部 VPS に Server)**: クラウド依存になり、医療データを院外に置くことの抵抗。
- **Cloudflare Workers + D1**: TypeScript 体制が混ざる。管理機能を含めると Workers の制約 (CPU time 等) に当たる可能性。

## 結論

院内 Raspberry Pi + Cloudflare Tunnel で C プラン全面採用。
