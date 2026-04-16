# ADR 0012: RC-S380 macOS USB プロトコル問題の調査記録

- **日付**: 2026-04-16
- **状態**: In Progress (調査中)
- **関連**: ADR 0011, docs/spec/08_rcs380_rusb_driver.md

## 問題

RC-S380 に SetCommandType コマンドを送信したとき、レスポンスフレームの形式が異常です。

### 観測されたデータ

```
送信: [00 00 FF FF FF 04 00 FC D6 2A 01 03 FC 00]
      (SetCommandType: D6 2A 01 03 をフレームでラップ)

受信:
  ✓ ACK: [00 00 FF 00 FF 00] (正常)
  ✗ Response: [00 00 FF 01 FF 7F 81 00] (異常)
```

### 異常の詳細

期待されるレスポンス形式（NFC Port-100）:
```
[00] [00] [FF] [FF] [FF] [LEN_LO] [LEN_HI] [LEN_CHK] [PAYLOAD...] [DATA_CHK] [00]
```

受け取ったデータの解釈試行:
```
[00] [00] [FF]       ← プリアンブル（正常）
[01] [FF]            ← 拡張マーカ？（異常。本来は [FF] [FF]）
[7F] [81]            ← LEN_LO/LEN_HI？
[00]                 ← ポストアンブル？

総バイト数: 8 bytes
期待最小長: 10 bytes (空ペイロードの場合)
```

## 原因の仮説

### 仮説 1: macOS libusb ドライバの制限

- macOS の libusb 経由では、カーネルドライバ（IOKit）の制限で、RC-S380 が正常に応答しない可能性
- PC/SC / ifd-ccid (CCID プロトコル) は macOS でサポートされているが、raw USB は不十分かもしれない

### 仮説 2: USB初期化シーケンスの不足

nfcpy (`rcs380.py`) の実装を見ると、単なる bulk transfer では足りず、以下が必要かもしれない：
- USB control transfer (SET_CONFIGURATION, SET_INTERFACE など)
- 複数の初期化コマンド（GetFirmwareVersion など）
- エンドポイント設定の詳細化

### 仮説 3: デバイスファームウェアの状態

- RC-S380 が何らかのエラー状態にある
- USB リセットが必要
- ファームウェアバージョンの非互換性

## 次のステップ（優先度順）

### Step 1: nfcpy の初期化シーケンスを詳しく調査

```python
# nfcpy rcs380.py から抜粋すべき部分：
- usb_open() と初期化フロー
- control_message() 呼び出しの詳細
- GetSystemInformation や GetFirmwareVersion コマンド
```

### Step 2: libusb デバッグモードの有効化

```bash
# libusb デバッグ情報を取得
export LIBUSB_DEBUG=4
cargo test -p terminal rcs380::transport::tests::hardware_usb_communication -- --ignored --nocapture
```

### Step 3: USB スニファーでの通信内容確認

- Wireshark / USB sniffer (macOS では Xcode の USB Prober)
- 実際のバイト列を取得して、何が起きているか確認

### Step 4: Linux / Windows での動作確認

もし Linux で同じコードが正常に動くなら、macOS 固有の問題と判定できる。

## 参考資料

- nfcpy rcs380.py: https://github.com/nfcpy/nfcpy/blob/master/src/nfc/clf/rcs380.py
- rusb documentation: https://docs.rs/rusb/
- USB Device Class Definition for Smart Card: USB CCID spec

## 一時的な回避案

- RC-S300 (CCID 互換) に変更する
- PC/SC 経由での利用に限定する（ただしmacOS では ifd-ccid に PID を追加する必要あり）
- Linux / Windows のみで RC-S380 を使用する
