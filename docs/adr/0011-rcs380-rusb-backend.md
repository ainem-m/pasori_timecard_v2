# ADR 0011: RC-S380 向け rusb バックエンド追加

- **日付**: 2026-04-16
- **状態**: Accepted
- **関連**: ADR 0001 (tech-stack), AGENTS.md §7.1, docs/spec/01_nfc_and_punch.md

## 背景

ADR 0001 で NFC は `pcsc` crate (PC/SC 経由) と確定した。
しかし、現場で使用する **Sony RC-S380 (PaSoRi)** は `bDeviceClass=255`
(Vendor-Specific) であり、macOS の `ifd-ccid.bundle` に PID `0x06C3` が
含まれていない。

調査結果:
- `ioreg` でデバイスは認識される (VID=0x054C, PID=0x06C3)
- `pcsc::Context::establish()` は成功する
- `ctx.list_readers()` が空リストを返す → ifd-ccid が RC-S380 を知らない
- macOS の ifd-ccid.bundle は RC-S660 (0x06C1) / RC-S300 (0x06C0) のみ対応
- RC-S380 は Sony 独自の **NFC Port-100** USB プロトコルで通信する必要がある

Linux では libnfc が RC-S380 をサポートするが、macOS/Windows では PC/SC 経由
では使えないため、USB 直接通信が唯一の選択肢。

## 決定

- `rusb` crate (libusb wrapper) を workspace dependencies に追加する
- `crates/terminal` に RC-S380 専用の **NFC Port-100 プロトコルドライバ** を実装する
- 既存の `PcscReaderBackend` は RC-S300/RC-S660 等の CCID 対応リーダー向けに残す
- **自動検出ファクトリ** (`detect_and_create()`) を提供し、接続デバイスに応じてバックエンドを自動選択する
- `core` crate の `ReaderBackend` trait は変更しない (hexagonal architecture 維持)
- `core` crate の `ReaderError` に `Usb(String)` と `Protocol(String)` バリアントを追加する

## 結果

- macOS/Windows/Linux で RC-S380 を使用した打刻が可能になる
- CCID 対応リーダー (RC-S300 等) も引き続きサポートされる
- `core` は `rusb` に依存しない (trait 境界のみ)

## 代替案と却下理由

- **ifd-ccid.bundle にエントリを手動追加**: macOS の SIP で保護された領域への書き込みが必要。配布時にユーザーに SIP 無効化を要求するのは非現実的
- **libnfc crate**: Rust バインディングが不安定、macOS サポートが弱い
- **nfcpy (Python) を subprocess で呼ぶ**: v1 から Rust に移行した意味がなくなる
- **RC-S300 に買い替え**: 既存ハードウェアを活用したい、RC-S380 は現場に複数台ある
