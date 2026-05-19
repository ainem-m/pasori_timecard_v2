# ADR 0014: MVP を PaSoRi 打刻と Terminal 側カード紐付けに絞る

- **日付**: 2026-05-19
- **状態**: Accepted
- **Supersedes**: ADR 0003 の MVP 範囲、ADR 0005 の LINE WORKS MVP 判断

## 背景

MVP の目的を再確認した結果、最初の実運用は **1 台 Server + 1 台 Terminal** で、
まず PaSoRi 打刻と Admin での勤怠確認を成立させることが最優先であると分かった。

一方、既存 ADR 0003 / 0005 は LINE WORKS 双方向、打刻修正、申請承認、シフト管理まで
MVP に含めており、初回リリースの検証範囲が広すぎる。

## 決定

MVP は以下に絞る。

- 1 台 Server
- 1 台 Terminal
- PaSoRi RC-S380 による登録済みカード打刻
- 未登録カード検出
- Terminal 側での未登録カード紐付け
- Admin Web での従業員追加・一覧・無効化
- Admin Web でのカード紐付け
- Admin Web での打刻一覧・月次勤怠確認
- オフライン打刻の local 保存と再送

LINE WORKS、打刻修正、申請承認、CSV 出力、締め処理、シフト管理は Phase 2 以降とする。
既存実装が存在しても、MVP UI では前面に出さない。

## Terminal 側カード紐付け

未登録カードを Terminal で読んだ場合、以下のフローを MVP 必須とする。

1. Terminal は Server へカード解決を問い合わせる
2. Server が未登録として返す
3. Terminal は Server から有効従業員一覧を取得する
4. Terminal は従業員氏名のみを一覧表示する
5. 操作者は従業員を選ぶ
6. Terminal は確認画面を表示する
7. 確認後、Terminal API token で Server に紐付けを要求する
8. Server は `card.bind` を audit_log に記録する
9. Terminal は local card cache に保存する
10. Terminal は「山田太郎に登録しました」と表示し、打刻せず待受に戻る

制約:

- 操作者認証は挟まない
- オンライン時のみ実行できる
- Server 不通時は「しばらくしてもう一度試してください」と表示する
- 紐付け失敗時は「もう一度試してください」と表示する
- カード ID は Terminal 画面に表示しない
- 有効従業員のみ選択できる
- 既に紐付いたカードは通常打刻し、Terminal から付け替えしない
- 1 従業員に複数カードを紐付けできる
- 1 カードは同時に 1 従業員だけに紐付く
- Admin のカード紐付け機能は事前登録・誤登録修正用に残す

## 監査

Terminal 側カード紐付けの audit_log は以下とする。

- `actor_type = "terminal"`
- `actor_id = terminal.id`
- `action = "card.bind"`
- `target_type = "card"`
- `target_id = card.id`
- `metadata_json.source = "terminal_unregistered_card_flow"`
- `metadata_json.employee_id = <employee id>`

操作者個人は追跡しない。MVP では認証なし運用の代わりに、Terminal 単位の監査と
Admin からの解除・付け替えでリスクを抑える。

## 理由

- 初回利用価値は「PaSoRi で打刻し、Admin で勤怠を確認できる」ことで成立する
- 未登録カードを Admin だけで紐付ける運用は現場導入時の摩擦が大きい
- 従業員数は約 20 人想定なので、Terminal での氏名一覧選択は MVP として十分
- LINE WORKS は目玉機能だが、外部サービス・Cloudflare Tunnel・署名検証・運用 secret が絡み、
  初回 PaSoRi 検証とは別フェーズにした方がリスクを分離できる

## 影響

- ADR 0003 の MVP 範囲は本 ADR で置き換える
- ADR 0005 の LINE WORKS MVP 判断は Phase 2 に延期する
- `docs/spec/01_nfc_and_punch.md` の未登録カード仕様を更新する
- `docs/spec/04_lineworks.md` は Phase 2 仕様として扱う
- `docs/spec/07_security.md` に Terminal token によるカード紐付け API を追加する
