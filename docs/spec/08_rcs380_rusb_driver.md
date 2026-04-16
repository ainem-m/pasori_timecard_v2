# 詳細仕様: RC-S380 rusb ドライバ実装計画

この文書は ADR 0011 に基づき、Sony RC-S380 (PaSoRi) 向け NFC Port-100
プロトコルドライバの実装手順を定義する。

**実装者向け注意**: この文書に記載されたバイト列・コマンドシーケンスは
nfcpy (rcs380.py) のリバースエンジニアリング結果に基づく。TDD で
1 ステップずつ検証しながら進めること。

---

## 1. 前提知識

### 1.1 デバイス情報

| 項目 | 値 |
|---|---|
| Vendor ID | `0x054C` (Sony) |
| Product ID | `0x06C3` (RC-S380) |
| USB Interface | 0 |
| Endpoint OUT | `0x02` (Bulk) |
| Endpoint IN | `0x81` (Bulk) |
| USB Timeout | 1000ms (通常)、2500ms (RF 通信) |

### 1.2 NFC Port-100 フレーム構造

全コマンド/レスポンスは以下のフレーム形式でラップされる:

```
[00] [00] [FF] [FF] [FF] [LEN_LO] [LEN_HI] [LEN_CHK] [PAYLOAD...] [DATA_CHK] [00]
```

| フィールド | サイズ | 説明 |
|---|---|---|
| プリアンブル | 3 bytes | 固定 `00 00 FF` |
| 拡張マーカ | 2 bytes | 固定 `FF FF` |
| LEN_LO | 1 byte | payload 長の下位バイト |
| LEN_HI | 1 byte | payload 長の上位バイト |
| LEN_CHK | 1 byte | `(LEN_LO + LEN_HI) & 0xFF` が `0x00` になるチェックサム |
| PAYLOAD | N bytes | コマンドデータ本体 |
| DATA_CHK | 1 byte | payload 全バイトの合計の 2 の補数 (下位 8 bit) |
| ポストアンブル | 1 byte | 固定 `00` |

### 1.3 コマンド/レスポンス構造

- **コマンド**: payload は `[D6, CMD, ...]` で始まる
- **レスポンス**: payload は `[D7, CMD+1, ...]` で始まる
- **ACK フレーム**: `[00, 00, FF, 00, FF, 00]` (6 bytes)。レスポンス前に必ず受信する

### 1.4 主要コマンド一覧

| コマンド名 | CMD byte | 用途 |
|---|---|---|
| SetCommandType | `0x2A` | 通信モード設定 |
| SwitchRF | `0x06` | RF アンテナ ON/OFF |
| InSetRF | `0x00` | RF 通信パラメータ設定 |
| InSetProtocol | `0x02` | プロトコルパラメータ設定 (タイムアウト等) |
| InCommRF | `0x04` | RF コマンド送受信 (FeliCa Polling 等) |

---

## 2. モジュール構成

```
crates/terminal/src/
├── reader.rs            # 既存 PcscReaderBackend + 新規 auto-detect factory
└── rcs380/
    ├── mod.rs           # RCS380ReaderBackend (ReaderBackend impl)
    ├── frame.rs         # NFC Port-100 フレームの encode/decode
    ├── transport.rs     # USB 送受信 (rusb wrapper) + Transport trait
    └── chipset.rs       # コマンドシーケンス (init, polling, cleanup)
```

---

## 3. ファイル別実装仕様

### 3.1 `crates/core/src/port/reader.rs` — ReaderError 拡張

`ReaderError` に 2 バリアント追加:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ReaderError {
    #[error("reader not connected")]
    NotConnected,
    #[error("pcsc error: {0}")]
    Pcsc(String),
    #[error("usb error: {0}")]
    Usb(String),           // ← 追加
    #[error("protocol error: {0}")]
    Protocol(String),      // ← 追加
    #[error("other: {0}")]
    Other(String),
}
```

**注意**: `core` は `rusb` に依存してはならない。エラーは `String` で受け取る。

### 3.2 `crates/terminal/src/rcs380/frame.rs` — フレーム処理

#### 公開 API

```rust
/// NFC Port-100 フレームをエンコードする。
/// payload は [D6, CMD, ...] 形式のコマンドデータ。
pub fn encode(payload: &[u8]) -> Vec<u8>;

/// 受信バイト列から NFC Port-100 フレームをデコードする。
/// ACK フレームの場合は Ok(None) を返す。
/// 正常フレームの場合は payload 部分を返す。
pub fn decode(data: &[u8]) -> Result<Option<Vec<u8>>, FrameError>;

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("frame too short: {0} bytes")]
    TooShort(usize),
    #[error("invalid preamble")]
    InvalidPreamble,
    #[error("length checksum mismatch")]
    LengthChecksumMismatch,
    #[error("data checksum mismatch")]
    DataChecksumMismatch,
}
```

#### encode 実装詳細

```
入力: payload (例: [D6, 2A, 01, 03])
出力:
  [00, 00, FF]           -- プリアンブル
  [FF, FF]               -- 拡張マーカ
  [04, 00]               -- LEN_LO=4, LEN_HI=0 (payload 長)
  [FC]                   -- LEN_CHK: 0x100 - (0x04 + 0x00) = 0xFC
  [D6, 2A, 01, 03]      -- payload
  [FC]                   -- DATA_CHK: (0x100 - (0xD6+0x2A+0x01+0x03)) & 0xFF
  [00]                   -- ポストアンブル
```

チェックサム計算:
- `LEN_CHK = (0x100 - ((len_lo as u16 + len_hi as u16) & 0xFF)) as u8 & 0xFF`
  - 実質: `LEN_LO + LEN_HI + LEN_CHK` の下位 8 bit が `0x00`
- `DATA_CHK = (0x100 - (payload の全バイト合計 & 0xFF)) as u8 & 0xFF`
  - 実質: `payload 全バイト + DATA_CHK` の下位 8 bit が `0x00`

#### decode 実装詳細

1. 長さチェック: 最小 6 bytes (ACK) または 11 bytes (データフレーム)
2. ACK 判定: `[00, 00, FF, 00, FF, 00]` に完全一致したら `Ok(None)`
3. プリアンブル検証: `data[0..3] == [0x00, 0x00, 0xFF]`
4. 拡張マーカ検証: `data[3..5] == [0xFF, 0xFF]`
5. 長さ取得: `len = data[5] as u16 | (data[6] as u16) << 8`
6. LEN_CHK 検証: `(data[5] + data[6] + data[7]) & 0xFF == 0`
7. payload 抽出: `data[8..8+len]`
8. DATA_CHK 検証: `(payload の合計 + data[8+len]) & 0xFF == 0`
9. `Ok(Some(payload.to_vec()))`

#### TDD テストケース

```
1. encode に空 payload → 正しいフレームが生成される
2. encode に [D6, 2A, 01, 03] → 手計算と一致するバイト列
3. decode(encode(payload)) で round-trip
4. decode に ACK フレーム → Ok(None)
5. decode に短すぎるデータ → TooShort エラー
6. decode に壊れたプリアンブル → InvalidPreamble エラー
7. decode に壊れた LEN_CHK → LengthChecksumMismatch エラー
8. decode に壊れた DATA_CHK → DataChecksumMismatch エラー
```

### 3.3 `crates/terminal/src/rcs380/transport.rs` — USB 通信

#### Transport trait (テスタビリティのため)

```rust
pub trait Transport: Send + Sync {
    /// フレームエンコード済みデータを送信する。
    fn send(&self, data: &[u8]) -> Result<(), TransportError>;

    /// 受信バッファにデータを読み取る。
    /// 戻り値は読み取ったバイト数。
    fn recv(&self, buf: &mut [u8], timeout_ms: u64) -> Result<usize, TransportError>;
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("usb error: {0}")]
    Usb(String),
    #[error("timeout")]
    Timeout,
    #[error("device not found")]
    DeviceNotFound,
}
```

#### UsbTransport 実装

```rust
pub struct UsbTransport {
    handle: rusb::DeviceHandle<rusb::GlobalContext>,
}

impl UsbTransport {
    /// RC-S380 を USB から検索して接続する。
    pub fn open() -> Result<Self, TransportError> {
        // 1. rusb::devices() でデバイス一覧取得
        // 2. VID=0x054C, PID=0x06C3 を検索
        // 3. open() → claim_interface(0)
        // 4. カーネルドライバが attach されていれば detach
    }
}

impl Transport for UsbTransport {
    fn send(&self, data: &[u8]) -> Result<(), TransportError> {
        // self.handle.write_bulk(0x02, data, Duration::from_millis(1000))
    }

    fn recv(&self, buf: &mut [u8], timeout_ms: u64) -> Result<usize, TransportError> {
        // self.handle.read_bulk(0x81, buf, Duration::from_millis(timeout_ms))
    }
}

impl Drop for UsbTransport {
    fn drop(&mut self) {
        // release_interface(0)
    }
}
```

#### MockTransport (テスト用)

```rust
#[cfg(test)]
pub struct MockTransport {
    /// recv() が返すデータのキュー。
    responses: std::sync::Mutex<std::collections::VecDeque<Vec<u8>>>,
    /// send() に渡されたデータの記録。
    sent: std::sync::Mutex<Vec<Vec<u8>>>,
}
```

### 3.4 `crates/terminal/src/rcs380/chipset.rs` — コマンドシーケンス

#### 公開 API

```rust
pub struct Chipset<T: Transport> {
    transport: T,
}

impl<T: Transport> Chipset<T> {
    pub fn new(transport: T) -> Self;

    /// チップセット初期化 (SetCommandType + SwitchRF)。
    pub fn initialize(&self) -> Result<(), ChipsetError>;

    /// FeliCa カードをポーリングし、IDm を取得する。
    /// カードが存在しない場合は Ok(None)。
    pub fn felica_polling(&self, system_code: u16) -> Result<Option<String>, ChipsetError>;

    /// RF をオフにしてクリーンアップ。
    pub fn shutdown(&self) -> Result<(), ChipsetError>;
}
```

#### initialize シーケンス

```
Step 1: SetCommandType (通信タイプ 3 = FeliCa)
  送信: frame::encode([D6, 2A, 01, 03])
  受信: ACK → レスポンスフレーム [D7, 2B, 00, ...] (status=0x00 で成功)

Step 2: SwitchRF (RF ON)
  送信: frame::encode([D6, 06, 00])
  受信: ACK → レスポンスフレーム [D7, 07, 00, ...] (status=0x00 で成功)

Step 3: InSetRF (212F パラメータ)
  送信: frame::encode([D6, 00, 01, 01, 0F, 01])
  受信: ACK → レスポンスフレーム [D7, 01, 00, ...]

Step 4: InSetProtocol (タイムアウト等)
  送信: frame::encode([
    D6, 02, 00,
    18, 01, 01,  // 初期タイムアウト
    18, 02, 07,  // タイムアウト値
    18, 03, 07,  // リトライタイムアウト
    18, 04, 00,  // リトライ回数
    18, 05, 00,  // 追加リトライ
  ])
  受信: ACK → レスポンスフレーム [D7, 03, 00, ...]
```

#### felica_polling シーケンス

```
Step 1: SENSF_REQ を InCommRF で送信
  SENSF_REQ: [06, 04, SC_HI, SC_LO, 01, 00]
    - 06 = SENSF_REQ の長さ
    - 04 = SENSF_REQ コマンドコード
    - SC_HI, SC_LO = system_code (例: 0xFFFF → [FF, FF])
    - 01 = リクエストコード (IDm + PMm 要求)
    - 00 = タイムスロット数

  InCommRF コマンド payload:
    [D6, 04, 6E, 00, 06, 04, SC_HI, SC_LO, 01, 00]
    - D6 = コマンドマーカ
    - 04 = InCommRF
    - 6E, 00 = 通信パラメータ
    - 残り = SENSF_REQ データ

  送信: frame::encode(上記 payload)

Step 2: レスポンス受信
  受信: ACK → レスポンスフレーム
  レスポンス payload: [D7, 05, STATUS, TIMEOUT, LEN, DATA...]

  - STATUS == 0x00 かつ LEN > 0: カード検出
    - DATA の構造: [LEN, 01, IDm(8bytes), PMm(8bytes), ...]
    - IDm = DATA[2..10] (8 バイト)
  - STATUS != 0x00 または LEN == 0: カード未検出 → Ok(None)

Step 3: IDm を hex 文字列に変換
  例: [01, FE, 01, 00, 11, 22, 33, 44] → "01FE010011223344"
```

#### shutdown シーケンス

```
SwitchRF (RF OFF):
  送信: frame::encode([D6, 06, 00])
  受信: ACK → レスポンスフレーム [D7, 07, 00, ...]
```

#### iPhone エクスプレスカード対応

iPhone の交通系 IC (Suica/PASMO) はエクスプレスカードモードで常時待受けている。
system code `0x0003` (交通系) でポーリングすると反応するが、`0xFFFF` (全カード)
では反応しないことがある。

対策: ポーリングを **交互に** 行う。

```
1 回目: felica_polling(0xFFFF)  // 通常の FeliCa カード
2 回目: felica_polling(0x0003)  // 交通系 IC / iPhone Express Card
3 回目: felica_polling(0xFFFF)
4 回目: felica_polling(0x0003)
...
```

この交互ポーリングは `RCS380ReaderBackend` の `poll_loop` 内で実装する。

#### TDD テストケース

```
1. initialize: MockTransport に正常レスポンスをセット → Ok(())
2. initialize: ステータス異常 → ChipsetError
3. felica_polling: カード検出レスポンス → Ok(Some("01FE..."))
4. felica_polling: カード未検出 → Ok(None)
5. felica_polling: タイムアウト → エラー
6. shutdown: 正常 → Ok(())
7. round-trip: initialize → polling → shutdown の完全シーケンス
```

### 3.5 `crates/terminal/src/rcs380/mod.rs` — ReaderBackend 実装

```rust
pub struct RCS380ReaderBackend {
    status: Arc<Mutex<ReaderStatus>>,
    tx: broadcast::Sender<CardScanned>,
    handle: Mutex<Option<JoinHandle<()>>>,
    cancel: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
}
```

構造は `PcscReaderBackend` とほぼ同じ。`start()` 内で:

1. `UsbTransport::open()` で RC-S380 に接続
2. `Chipset::new(transport)` で chipset 初期化
3. `tokio::task::spawn_blocking` でポーリングループ起動

ポーリングループ:

```rust
fn poll_loop(
    chipset: Chipset<UsbTransport>,
    tx: broadcast::Sender<CardScanned>,
    status: Arc<Mutex<ReaderStatus>>,
    cancel: tokio::sync::watch::Receiver<bool>,
) {
    if let Err(e) = chipset.initialize() {
        // status を Error に設定して return
    }
    // status を Ready に設定

    let mut last_seen: HashMap<String, std::time::Instant> = HashMap::new();
    let mut use_transport_system_code = false; // 交互ポーリング用フラグ

    loop {
        if *cancel.borrow() { break; }

        let system_code = if use_transport_system_code { 0x0003 } else { 0xFFFF };
        use_transport_system_code = !use_transport_system_code;

        match chipset.felica_polling(system_code) {
            Ok(Some(idm_hex)) => {
                // 連続スキャン抑制 (5 秒)
                // CardScanned を broadcast
            }
            Ok(None) => {}
            Err(e) => { tracing::debug!("polling error: {e}"); }
        }

        std::thread::sleep(Duration::from_millis(200));
    }

    let _ = chipset.shutdown();
}
```

### 3.6 `crates/terminal/src/reader.rs` — 自動検出ファクトリ

既存の `PcscReaderBackend` に加え、自動検出関数を追加:

```rust
use crate::rcs380::RCS380ReaderBackend;

/// 接続されている NFC リーダーを自動検出し、適切なバックエンドを返す。
///
/// 検出順序:
/// 1. USB デバイス一覧から RC-S380 (VID=054C, PID=06C3) を検索 → RCS380ReaderBackend
/// 2. PC/SC リーダー一覧を検索 → PcscReaderBackend
/// 3. いずれも見つからない場合 → ReaderError::NotConnected
pub fn detect_and_create() -> Result<Box<dyn ReaderBackend>, ReaderError> {
    // 1. rusb でデバイス検索
    if rusb::devices()
        .map(|list| list.iter().any(|d| {
            d.device_descriptor().map_or(false, |desc| {
                desc.vendor_id() == 0x054C && desc.product_id() == 0x06C3
            })
        }))
        .unwrap_or(false)
    {
        tracing::info!("RC-S380 detected, using rusb backend");
        return Ok(Box::new(RCS380ReaderBackend::new()));
    }

    // 2. PC/SC フォールバック
    if let Ok(ctx) = pcsc::Context::establish(pcsc::Scope::User) {
        let mut buf = vec![0u8; 4096];
        if ctx.list_readers(&mut buf).map_or(false, |mut r| r.next().is_some()) {
            tracing::info!("PC/SC reader detected, using pcsc backend");
            return Ok(Box::new(PcscReaderBackend::new()));
        }
    }

    Err(ReaderError::NotConnected)
}
```

### 3.7 `crates/terminal/src/main.rs` — エントリポイント変更

```rust
// 変更前:
let backend = PcscReaderBackend::new();

// 変更後:
let backend = reader::detect_and_create()?;
```

### 3.8 `Cargo.toml` (workspace) — 依存追加

```toml
[workspace.dependencies]
rusb = "0.9"
```

### 3.9 `crates/terminal/Cargo.toml` — 依存追加

```toml
[dependencies]
rusb.workspace = true
```

---

## 4. 実装順序 (TDD)

各ステップで Red → Green → Refactor サイクルを回す。

### Phase 1: フレーム処理 (`frame.rs`)

**ここが最も独立していて、USB 接続なしでテスト可能。最初に着手する。**

```
TODO:
[ ] frame::encode — 空 payload
[ ] frame::encode — [D6, 2A, 01, 03] で手計算一致
[ ] frame::decode — ACK フレーム → Ok(None)
[ ] frame::decode — 正常データフレーム → Ok(Some(payload))
[ ] frame::decode — round-trip (encode → decode)
[ ] frame::decode — TooShort エラー
[ ] frame::decode — InvalidPreamble エラー
[ ] frame::decode — LengthChecksumMismatch エラー
[ ] frame::decode — DataChecksumMismatch エラー
```

### Phase 2: Transport trait + MockTransport (`transport.rs`)

```
TODO:
[ ] Transport trait 定義
[ ] MockTransport 実装
[ ] UsbTransport::open — デバイス検索ロジック (実機テストは手動)
[ ] UsbTransport の send/recv (実機テストは手動)
```

### Phase 3: Chipset コマンド (`chipset.rs`)

**MockTransport を使い、送信バイト列とレスポンス処理をテスト。**

```
TODO:
[ ] Chipset::initialize — 正常系 (4 コマンド分の ACK + レスポンスを MockTransport にセット)
[ ] Chipset::initialize — 途中でエラー → ChipsetError
[ ] Chipset::felica_polling — カード検出 → Ok(Some(idm_hex))
[ ] Chipset::felica_polling — カード未検出 → Ok(None)
[ ] Chipset::shutdown — 正常系
[ ] initialize → polling → shutdown の統合テスト
```

### Phase 4: ReaderBackend 実装 (`rcs380/mod.rs`)

```
TODO:
[ ] RCS380ReaderBackend::new — 初期状態は Disconnected
[ ] subscribe — Receiver を返す
[ ] (実機テスト) start → status が Ready に遷移
[ ] (実機テスト) カードタッチで CardScanned イベント受信
[ ] stop → status が Disconnected に遷移
```

### Phase 5: ReaderError 拡張 + 自動検出 (`reader.rs`)

```
TODO:
[ ] core の ReaderError に Usb / Protocol バリアント追加
[ ] detect_and_create — RC-S380 接続時は RCS380ReaderBackend
[ ] detect_and_create — PC/SC リーダー接続時は PcscReaderBackend
[ ] detect_and_create — 未接続時は NotConnected エラー
[ ] main.rs をファクトリ呼び出しに変更
```

### Phase 6: iPhone エクスプレスカード対応

```
TODO:
[ ] poll_loop の交互ポーリング (0xFFFF / 0x0003)
[ ] (実機テスト) iPhone の Suica を検出できる
```

---

## 5. 既存コードの修正箇所

### `expect()` の除去

`crates/terminal/src/reader.rs` の `PcscReaderBackend` には `expect()` が
複数残っている (CLAUDE.md §12 違反)。rusb 実装と合わせて修正する:

- `self.status.lock().expect("status lock")` → `match` or `map_err`
- `self.handle.lock().expect("handle lock")` → 同上
- `self.cancel.lock().expect("cancel lock")` → 同上

### ReaderError の Serialize/Deserialize

現在 `ReaderError` は Serialize/Deserialize を derive していないが、
`Usb` / `Protocol` 追加後も String ベースなので問題なし。

---

## 6. テスト戦略

| レイヤ | テスト手法 | CI で実行 |
|---|---|---|
| `frame.rs` | 純粋関数の単体テスト | Yes |
| `transport.rs` (MockTransport) | mock ベースの単体テスト | Yes |
| `chipset.rs` | MockTransport で統合テスト | Yes |
| `rcs380/mod.rs` | 初期状態テスト (mock) | Yes |
| USB 実通信 | `#[ignore]` + 手動実行 | No (実機必要) |
| カード読取 | `#[ignore]` + 手動実行 | No (実機+カード必要) |

`#[ignore]` テストには `docs/adr/` で理由を明記する (CLAUDE.md §12 準拠)。

実機テストの実行方法:

```bash
# RC-S380 を USB 接続した状態で:
cargo test -p terminal -- --ignored rcs380
```

---

## 7. 参考資料

- nfcpy `rcs380.py`: NFC Port-100 プロトコルのリファレンス実装
  - https://github.com/nfcpy/nfcpy/blob/master/src/nfc/clf/rcs380.py
- USB PID/VID: https://devicehunt.com/view/type/usb/vendor/054C
- FeliCa 技術仕様: JIS X 6319-4 (SENSF_REQ/RES)
