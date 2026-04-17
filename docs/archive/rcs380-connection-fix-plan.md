# 実装計画: RC-S380 macOS USB 接続修正

- **日付**: 2026-04-17
- **対象**: `crates/terminal/src/rcs380/` 一式
- **関連**: `chatgpt.md`（修正版 ADR 0012）、`docs/adr/0012-rcs380-macos-usb-protocol-issue.md`、`docs/spec/08_rcs380_rusb_driver.md`
- **ゴール**: RC-S380 でのカード読み取りをエンドツーエンドで成功させる（macOS 実機で `hardware_full_cycle` が green になる）
- **前提**: `chatgpt.md` の解析結果を「確定仕様」として採用する。すなわち、現在の frame parser は **通常フレームを処理できない** ことが原因。

> **下位モデルへ**: この計画は TDD 規約 (`CLAUDE.md §6`、ADR 0004) を厳守して進めること。
> 各 Phase の末尾にある「検証コマンド」が全て green になってから次の Phase に進む。
> Red → Green → Refactor サイクルを**コミット単位**で残すこと。

---

## 0. 背景（必読）

### 0.1 現状の挙動
`cargo test -p terminal rcs380 -- --ignored hardware_full_cycle` を macOS 実機で実行すると:

```
送信:   [00 00 FF FF FF 04 00 FC D6 2A 01 03 FC 00]   (SetCommandType)
受信 1: [00 00 FF 00 FF 00]                           (ACK, 正常)
受信 2: [00 00 FF 01 FF 7F 81 00]                     (8 bytes)
```

現在の `frame::decode()` は受信 2 を処理できず、`FrameError::TooShort(8)` または `InvalidPreamble` で失敗する。そのため `chipset::initialize()` が失敗し、以降のポーリングに到達しない。

### 0.2 受信 2 の正しい解釈
`chatgpt.md` の再解析により、受信 2 は「破損」ではなく **整合した通常フレーム**である:

| バイト | 値 | 意味 |
|---|---|---|
| data[0..3] | `00 00 FF` | preamble |
| data[3] | `01` | LEN |
| data[4] | `FF` | LCS (`LEN + LCS = 0 mod 256`) |
| data[5] | `7F` | payload (1 byte) |
| data[6] | `81` | DCS (`payload + DCS = 0 mod 256`) |
| data[7] | `00` | postamble |

さらに、このバイト列 `[00 00 FF 01 FF 7F 81 00]` は PN532 / NFC Port ファミリの仕様上、**Error Frame**（"specific application level error" を示す予約パターン）と一致する。つまり:

> USB 通信は完全に健全。RC-S380 は「ホストが送った SetCommandType を、現状のステートでは実行できない」旨のエラーを**プロトコル準拠の形式で**返している。

### 0.3 確定している 2 つの欠陥

1. **パーサの欠陥** (最優先): `frame::decode()` が拡張フレーム (`[FF FF]` マーカ) のみを前提にしており、通常フレーム (LEN=1 などの短いフレーム) を受理できない。また、最小長を 10 bytes と見積もっているため、8 bytes の Error Frame が `TooShort` になる。
2. **コマンドシーケンスの妥当性が未検証**: `SetCommandType` の引数 `0x03` が正しいか、初期化コマンドの順序が正しいか、`nfcpy` との比較なしに確定していない。Error Frame が返ってくること自体が「何かが無効」であることを示唆している。

### 0.4 スコープ外（この計画では触らない）

- PC/SC フォールバックの変更（`detect_and_create` は既に動いている）
- Linux / Windows での挙動確認（macOS 修正後に別タスク）
- `tracing` ログの整備（必要最小限のみ）
- `core::ReaderError` の追加バリアント（既存の `Protocol(String)` で足りる）

---

## 1. Phase 構成（実装順）

| Phase | 内容 | 検証 | ブロッキング |
|---|---|---|---|
| A | フレームパーサ拡張（通常フレーム + Error Frame） | `cargo test -p terminal rcs380::frame` | 後続の前提 |
| B | Chipset レイヤでの Error Frame ハンドリング | `cargo test -p terminal rcs380::chipset` | — |
| C | 初期化シーケンス見直し（GetFirmwareVersion を先頭に追加） | mock テスト | — |
| D | macOS 実機検証 | `hardware_full_cycle -- --ignored` | Phase A–C 完了 |
| E | ドキュメント更新（ADR 0012 のステータス遷移、memory 更新） | manual | Phase D 完了 |

---

## 2. Phase A: フレームパーサ拡張（最優先）

### 2.1 ゴール
`frame::decode()` が以下の 4 種類を **全て** 正しく識別できる:

| 種別 | バイト列 | 戻り値（案） |
|---|---|---|
| ACK | `00 00 FF 00 FF 00` (6B) | `Ok(DecodedFrame::Ack)` |
| Error | `00 00 FF 01 FF 7F 81 00` (8B) | `Ok(DecodedFrame::Error)` |
| Normal | `00 00 FF LEN LCS PAYLOAD... DCS 00` (≥ 8B, LEN < 0xFF) | `Ok(DecodedFrame::Data(payload))` |
| Extended | `00 00 FF FF FF LEN_LO LEN_HI LEN_CHK PAYLOAD... DCS 00` (≥ 10B) | `Ok(DecodedFrame::Data(payload))` |

### 2.2 API 変更案

**破壊的変更になる**ので、`Option<Vec<u8>>` を返す現 API を `Result<DecodedFrame, FrameError>` に置き換える。

```rust
// crates/terminal/src/rcs380/frame.rs

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedFrame {
    /// ACK フレーム (データなし)
    Ack,
    /// デバイスからの application-level error (payload = [0x7F])
    Error,
    /// 通常 / 拡張フレームの payload
    Data(Vec<u8>),
}

pub fn decode(data: &[u8]) -> Result<DecodedFrame, FrameError> { ... }
```

呼び出し側（`chipset.rs`）の `match payload { Some(p) => ..., None => ... }` を
`match decoded { DecodedFrame::Data(p) => ..., DecodedFrame::Ack => ..., DecodedFrame::Error => ... }` に置き換える。

### 2.3 TDD TODO リスト（Phase A）

> 以下を 1 つずつ Red → Green → Refactor で進めること。
> 各テスト関数名は英語 ASCII snake_case、**直前の日本語コメントで仕様を表現**（ADR 0009）。

- [ ] A-1. `DecodedFrame` enum を定義する（Ack / Error / Data）
- [ ] A-2. ACK フレームを `DecodedFrame::Ack` として返す（既存テスト `decodes_ack_frame` を新 API に合わせて更新）
- [ ] A-3. **新規テスト**: `[00 00 FF 01 FF 7F 81 00]` を `DecodedFrame::Error` として返す
  - テスト名: `decodes_application_error_frame`
  - 日本語コメント: `// [00 00 FF 01 FF 7F 81 00] は application-level error フレームとして識別される。`
- [ ] A-4. **新規テスト**: LEN=1, payload=[0x7E] の通常フレームを `DecodedFrame::Data(vec![0x7E])` として返す（Error 以外の通常フレーム）
  - テスト名: `decodes_normal_frame_with_single_byte_payload`
  - バイト列手計算:
    - preamble: `00 00 FF`
    - LEN=0x01, LCS=0xFF（`01+FF=0x100`）
    - payload=`7E`, DCS=`82`（`7E+82=0x100`）
    - postamble: `00`
- [ ] A-5. **新規テスト**: LEN=5 の通常フレームを payload 配列として返す
  - テスト名: `decodes_normal_frame_with_multibyte_payload`
  - payload 例: `[0xD7, 0x0B, 0x00, 0x01, 0x02]`
- [ ] A-6. 既存の拡張フレームテスト (`roundtrip_encode_decode`, etc.) が新 API でも通ることを確認・修正
- [ ] A-7. 新規テスト: 通常フレームの LCS 不整合 → `LengthChecksumMismatch`
- [ ] A-8. 新規テスト: 通常フレームの DCS 不整合 → `DataChecksumMismatch`
- [ ] A-9. 新規テスト: 通常フレームで `LEN > 0` かつ `data.len() < 5 + LEN + 2` → `TooShort`

### 2.4 実装ガイド（疑似コード）

```rust
pub fn decode(data: &[u8]) -> Result<DecodedFrame, FrameError> {
    // ACK (6 bytes, 固定パターン)
    if data.len() >= 6 && data[..6] == [0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00] {
        return Ok(DecodedFrame::Ack);
    }

    // Error Frame (8 bytes, 固定パターン) — 通常フレームの特殊ケースだが、先に判定して意味づけする
    if data.len() >= 8 && data[..8] == [0x00, 0x00, 0xFF, 0x01, 0xFF, 0x7F, 0x81, 0x00] {
        return Ok(DecodedFrame::Error);
    }

    if data.len() < 6 {
        return Err(FrameError::TooShort(data.len()));
    }
    if data[..3] != [0x00, 0x00, 0xFF] {
        return Err(FrameError::InvalidPreamble);
    }

    // 拡張 or 通常の分岐
    if data.len() >= 5 && data[3] == 0xFF && data[4] == 0xFF {
        // === 拡張フレーム ===
        // 既存ロジックをそのまま (data[5..] を LEN_LO/LEN_HI/LEN_CHK/payload.../DCS/00 として処理)
        decode_extended(data)
    } else {
        // === 通常フレーム ===
        // LEN = data[3], LCS = data[4]
        // LEN + LCS = 0 mod 256
        // payload = data[5..5+LEN]
        // DCS = data[5+LEN]
        // postamble = data[5+LEN+1]
        decode_normal(data)
    }
}

fn decode_normal(data: &[u8]) -> Result<DecodedFrame, FrameError> {
    let len = data[3] as usize;
    let lcs = data[4];
    if ((data[3] as u16 + lcs as u16) & 0xFF) != 0 {
        return Err(FrameError::LengthChecksumMismatch);
    }
    let total = 3 + 2 + len + 1 + 1; // preamble + LEN/LCS + payload + DCS + postamble
    if data.len() < total {
        return Err(FrameError::TooShort(data.len()));
    }
    let payload = &data[5..5 + len];
    let dcs = data[5 + len];
    let sum: u16 = payload.iter().map(|b| *b as u16).sum();
    if ((sum + dcs as u16) & 0xFF) != 0 {
        return Err(FrameError::DataChecksumMismatch);
    }
    Ok(DecodedFrame::Data(payload.to_vec()))
}
```

### 2.5 検証コマンド（Phase A 終了条件）

```bash
cargo fmt --all -- --check
cargo clippy -p terminal --all-targets -- -D warnings
cargo test -p terminal rcs380::frame
```

全て green になるまで Phase B に進まない。

---

## 3. Phase B: Chipset レイヤでの Error Frame 対応

### 3.1 ゴール

`chipset::send_command_and_recv()` が `DecodedFrame::Error` を受け取った際に、**明示的なエラー種別**で返す。これにより「コマンドは届いたが、デバイス側で拒否された」ことがログから一目で分かるようにする。

### 3.2 変更対象

- `crates/terminal/src/rcs380/chipset.rs`
  - `ChipsetError` に `#[error("device rejected command (application error frame)")] DeviceRejected` を追加
  - `send_command_and_recv` の `match decoded` で `DecodedFrame::Error => Err(ChipsetError::DeviceRejected)` を追加
  - `DecodedFrame::Ack => Err(ChipsetError::Protocol("expected data frame, got ACK".into()))` は既存相当を移植
  - `DecodedFrame::Data(p) => { /* 既存のステータス検査ロジック */ }`

### 3.3 TDD TODO（Phase B）

- [ ] B-1. `ChipsetError::DeviceRejected` バリアントを追加
- [ ] B-2. **新規テスト**: MockTransport に ACK + Error Frame をキューして `send_command_and_recv` を呼ぶと `Err(ChipsetError::DeviceRejected)` になる
  - テスト名: `send_command_returns_device_rejected_for_error_frame`
  - 日本語コメント: `// デバイスが Error Frame を返したら DeviceRejected として扱う。`
- [ ] B-3. 既存テスト `chipset_initialize_success` / `chipset_felica_polling_card_detected` / `chipset_felica_polling_no_card` が新 API (`DecodedFrame`) で green になることを確認
- [ ] B-4. （任意）`tracing::warn!` で Error Frame 受信時にコマンドバイト列をログに残す

### 3.4 検証コマンド（Phase B 終了条件）

```bash
cargo test -p terminal rcs380::chipset
cargo test -p terminal rcs380  # mod.rs も含めて全通過
```

---

## 4. Phase C: 初期化シーケンス見直し

> **重要**: Phase A/B が完了すると、実機テストで Error Frame を受け取った時点で `DeviceRejected` として正しく報告されるようになる。この時点で「USB 通信自体は健全」ということが確認できる。
> そのうえで、**なぜ SetCommandType(0x03) が Error Frame を誘発するのか** を切り分けるため、GetFirmwareVersion を先に送る戦略を採る。

### 4.1 GetFirmwareVersion の追加

nfcpy (`rcs380.py`) の初期化ロジックでは、USB open 直後に `GetFirmwareVersion` を呼んで疎通確認する。これは副作用がなく、デバイス状態に依存しない「純粋な読み出し」コマンドなので、疎通切り分けに最適。

#### コマンド仕様

- コマンドバイト: `[0xD6, 0x20]`（引数なし）
- レスポンス payload: `[0xD7, 0x21, ver_minor, ver_major]`（推定）

> **下位モデルへ**: nfcpy の `rcs380.py` を https://github.com/nfcpy/nfcpy/blob/master/src/nfc/clf/rcs380.py で開き、`get_firmware_version` メソッドの該当行を **必ず確認** してから実装すること。コマンドバイトと引数が上記推定と食い違う場合は、nfcpy を真として採用する。

### 4.2 実装計画

- `chipset.rs` に `pub fn get_firmware_version(&self) -> Result<String, ChipsetError>` を追加
  - `send_command_and_recv(&[0xD6, 0x20])` を呼ぶ
  - payload から `"{major}.{minor:02}"` 形式で文字列化して返す
- `initialize()` の**先頭**に `get_firmware_version()` を追加
  - 戻り値は `tracing::info!(fw = %ver, "RC-S380 firmware version")` で記録
  - 失敗時は `ChipsetError` をそのまま return（= 以降の初期化を試行しない）

### 4.3 TDD TODO（Phase C）

- [ ] C-1. **新規テスト**: mock に `[D7, 21, 0A, 01]` を返させて `get_firmware_version` が `"1.0A"` 等を返す
  - テスト名: `get_firmware_version_returns_version_string`
- [ ] C-2. **新規テスト**: `initialize` 先頭で `get_firmware_version` を呼び、失敗すると `initialize` 全体が失敗する
  - テスト名: `initialize_fails_when_get_firmware_version_fails`
- [ ] C-3. 既存 `chipset_initialize_success` のキューに firmware version レスポンスを 1 組追加（5 コマンド分の ACK + response）

### 4.4 検証コマンド（Phase C 終了条件）

```bash
cargo test -p terminal rcs380
```

---

## 5. Phase D: macOS 実機検証

### 5.1 前提
- RC-S380 を USB 接続
- 他プロセス（Python nfcpy 等）が `claim_interface` していない
- FeliCa カード（Suica / PASMO / 社員証等）を用意

### 5.2 実行手順

```bash
# 1. libusb デバッグを有効化して実行
export LIBUSB_DEBUG=4
cargo test -p terminal rcs380::chipset::tests::hardware_full_cycle -- --ignored --nocapture \
  2> libusb.log
```

### 5.3 期待される出力パターン

#### パターン 1: GetFirmwareVersion が成功する場合（最有力）
```
✓ RC-S380接続確認
  RC-S380 firmware version: 1.XX
✓ チップセット初期化成功
カードをタッチしてください...
✓ カード検出: IDm=XXXXXXXXXXXXXXXX
✓ シャットダウン成功
```
→ 完了。Phase E へ。

#### パターン 2: GetFirmwareVersion で `DeviceRejected`
→ USB 通信は届いているがデバイスが reset を要求している可能性。次節 5.4 を試す。

#### パターン 3: GetFirmwareVersion 成功、SetCommandType で `DeviceRejected`
→ `SetCommandType` の引数 `0x03` が不正の可能性。`0x01` に変更して再試行:
```rust
// chipset.rs:35
self.send_command_and_recv(&[0xD6, 0x2A, 0x01, 0x01])?;
```
（nfcpy は RC-S380 を Port-100 モードにするために `command_type=1` を渡す実装になっている。ただし nfcpy の現行コードを必ず確認すること。）

### 5.4 それでもダメな場合の切り分け（Phase D サブタスク）

下位モデルがここで詰まったら、以下を**順に**試して結果をログに残す:

- [ ] D-a. `UsbTransport::open()` 直後に `handle.reset()` を呼ぶ
- [ ] D-b. `handle.set_active_configuration(1)` を `claim_interface` の前に挿入
- [ ] D-c. USB control transfer (`SET_INTERFACE`) を明示的に発行（rusb の `write_control`）
- [ ] D-d. `send` 直前にホスト側から ACK フレーム `[00 00 FF 00 FF 00]` を送ってデバイスステートをリセット（nfcpy の `Chipset.send_command` にある abort 相当）

各試行の結果（成功/失敗、受信バイト列）を `docs/adr/0012-rcs380-macos-usb-protocol-issue.md` に追記すること。

---

## 6. Phase E: ドキュメント更新

### 6.1 ADR 0012 の更新
- ステータス `In Progress` → `Resolved`（Phase D でカード検出成功した場合）
- 「原因」「解決策」セクションを更新:
  - 原因: フレームパーサが通常フレームを認識できず、Error Frame を "malformed" と誤判定していた
  - 解決: `decode` を通常/拡張の両対応に拡張し、Error Frame を明示的に扱うようにした
  - （Phase D で追加修正が必要だった場合はそれも記載）

### 6.2 memory の更新
`/Users/ainem/.claude/projects/-Users-ainem-pasori-timecard-v2/memory/rc380_implementation.md` を更新:
- 実機テストセクションを「✓ macOS で動作確認 (2026-04-XX)」に書き換え
- 「既知の問題」セクションを削除または `Resolved` に変更

### 6.3 spec の微修正
`docs/spec/08_rcs380_rusb_driver.md` の frame.rs セクション (§3.2) を更新:
- `decode` の戻り値を `Result<DecodedFrame, FrameError>` に変更
- 通常フレームもサポートすることを明記
- Error Frame の扱いを追記

---

## 7. Definition of Done（この計画全体）

- [ ] Phase A〜E の全タスクが完了
- [ ] `cargo fmt --all -- --check` green
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` green
- [ ] `cargo test --workspace` green（実機テスト `#[ignore]` を除く）
- [ ] macOS 実機で `hardware_full_cycle` が green（カード IDm が出力される）
- [ ] ADR 0012 / memory / spec 08 が更新されている
- [ ] コミットは Conventional Commits に準拠し、Phase ごとに 1〜2 個に分かれている
  - 例: `refactor(terminal): extend rcs380 frame decoder to support normal frames`
  - 例: `feat(terminal): add GetFirmwareVersion to rcs380 initialization`
  - 例: `fix(terminal): handle application error frames as ChipsetError::DeviceRejected`

---

## 8. 下位モデル向けチェックリスト

実装開始前に以下を確認すること:

- [ ] `CLAUDE.md` を読んだ（特に §6 TDD 規約、§12 禁止事項）
- [ ] `chatgpt.md` を読んだ（フレーム解析の確定事項）
- [ ] `docs/adr/0012-rcs380-macos-usb-protocol-issue.md` を読んだ
- [ ] nfcpy の `rcs380.py` で `get_firmware_version` と `set_command_type` の実装を確認した
- [ ] `crates/terminal/src/rcs380/{frame.rs, transport.rs, chipset.rs, mod.rs}` の現状コードを通読した
- [ ] Phase A の TODO リストを自分の作業環境（TaskCreate 等）に取り込んだ

実装中に迷ったら:
- **仕様が読み取れない箇所がある** → 実装を止めて ADR を書く（CLAUDE.md §5）
- **既存テストが壊れた** → API 変更に伴う更新か、真の回帰かを見極める。回帰なら Phase をロールバック
- **実機テストで想定外の挙動** → §5.4 の切り分けを順に試し、結果を ADR 0012 に追記してから次へ

最後のコミット後に必ず `plan.md` (本ファイル) 自体を **`Resolved` に書き換えるか削除** すること（計画が陳腐化して残ると混乱の元になる）。
