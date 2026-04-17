結論から言うと、いちばん可能性が高い根本原因は **RC-S380 が壊れているのではなく、macOS 側の USB セッション確立手順か、Port-100 コマンドモードへの遷移が nfcpy/既知実装と一致していないこと** です。
特に重要なのは、既知の成功実装では **起動直後に ACK → `SetCommandType 01` → `GetFirmwareVersion` → `GetPDDataVersion` → `SwitchRF 00`** という順で進み、`SetCommandType 01` の成功応答は `0000ffffff0300fdd72b00fe00`、`GetFirmwareVersion` の成功応答は `0000ffffff0400fcd7211101f600` になっている点です。あなたの `0000ff01ff7f8100` は、この成功パターンと明確に異なります。 ([GitHub][1])

### まず、観測値の意味

あなたが受けている

```text
00 00 FF 01 FF 7F 81 00
```

は、フレームとしては壊れていません。
これは **通常フレーム** としてチェックサムが整合しており、`payload = 0x7F` の 1 バイト応答です。したがって、これは「USB 転送が乱れてゴミが返ってきた」より、**デバイスが意味のあるエラー/状態コードを返している** とみるべきです。公開されている WebUSB/nfcpy 系の成功例では、Port-100 の正常なコマンド応答は `D7 <cmd+1> ...` を含む拡張フレームで返っているため、`0x7F` 単独は成功応答ではありません。 ([GitHub][1])

ここで正直に言うと、**公開 web 上で確認できる資料の範囲では、Port-100 の `0x7F` 単独ステータスの公式な意味までは特定できませんでした**。Sony の仕様書も、詳細なコマンド意味は「SDK for NFC Reference Implementation 付属のコマンドリファレンスマニュアルを参照」としており、公開 PDF だけでは status code の一覧までは読めません。 

### 1. `payload = 0x7F` は何か

断定はできませんが、現状の証拠からは **Port-100 側の generic error / invalid state / invalid mode 系の応答** と考えるのが妥当です。
理由は単純で、成功時には `SetCommandType` でも `GetFirmwareVersion` でも `D7...` を含む拡張フレームが返るのに、あなたのケースでは **コマンド種類に関係なく常に `0x7F` だけ返る** からです。これは「特定コマンドの個別エラー」より、**セッションの前提状態が違う** と読む方が自然です。 ([GitHub][1])

### 2. macOS + libusb で特別な初期化が必要か

**少なくとも既知の成功実装では、特別な vendor control transfer は見えていません。**
一方で、**`configuration 1` を明示的に選択してから interface 0 を claim** しています。WebUSB の実装では `device.open()` のあとに `selectConfiguration(1)`、続いて `claimInterface(0)` を行っています。これはあなたの記述の「descriptor dump では Configuration:1 がある」だけでは代替できません。**“存在する” と “active になっている” は別です。** ([GitHub][1])

なので、Rust/rusb 側では最低限これを厳密に合わせるべきです。

```rust
handle.set_active_configuration(1)?;
handle.claim_interface(0)?;
```

この 2 行が既に入っていないなら、**最優先で追加**です。
`claim_interface(0)` が成功していても、macOS/libusb の実装差で active configuration の明示が効く余地があります。libusb の macOS 実装には `SetAlternateInterface` や configuration 周りの既知トラブル報告もありますが、今回の RC-S380 は alt setting を使わず、既知の成功例も `configuration 1 + interface 0` だけです。 ([GitHub][1])

### 3. 電源サイクル後も同じ `0x7F` が返る原因

この条件だと、原因候補はかなり絞れます。

1つ目は **USB セッションの作り方が違う** ことです。
とくに `set_active_configuration(1)` の欠落が最重要候補です。成功実装では明示されています。 ([GitHub][1])

2つ目は **Port-100 の開始シーケンスが違う** ことです。
既知の成功ログでは、最初に ACK を投げたあと、**最初の本コマンドが `SetCommandType 01`** です。その送信フレームは

```text
0000ffffff0300fdd62a01ff00
```

で、成功応答は

```text
0000ffffff0300fdd72b00fe00
```

です。あなたのログに出ていた `D6 2A 01 03` は nfcpy 互換ではありません。`SetCommandType(1)` は `D6 2A 01` が payload です。もし今は修正済みでも、**比較対象はこの exact bytes** に揃えるべきです。 ([GitHub][1])

3つ目は **他プロセス/他スタックによる干渉** です。
nfcpy は RC-S380 を libusb 経由で扱う前提で、起動時にまず ACK を投げて「他プロセスに claim されていないか」を見ています。あなたも `pcscd` を止めていますが、macOS 側で別のカードサービスやアプリが reopen している可能性は残ります。とはいえ、毎回同一の `0x7F` になるなら、競合より **初期化差** の方が有力です。 ([GitHub][2])

4つ目は **macOS + libusb の相性問題** です。
これはゼロではありません。libusb の macOS には既知の問題がいくつかあります。ただし、**RC-S380 自体が macOS で絶対に direct USB 不可という証拠はありません**。Sony の RC-S380/S 仕様書は「本製品は Windows, macOS, Linux, Android で動作可能」としており、nfcpy も RC-S380/S と RC-S380/P をサポート対象に挙げています。さらに、macOS 上で nfcpy + libusb を使った実動記事もあります。 

### 4. nfcpy は macOS で RC-S380 を direct USB で動かせるか

**はい、少なくとも「原理的に無理」ではありません。**
nfcpy の supported devices には `RC-S380/S usb:054c:06c1` と `RC-S380/P usb:054c:06c3` が載っています。`/P` は testbed ではないものの、脚注では `/S` との既知差は CE/FCC マーク程度とされています。macOS で nfcpy + libusb を使って RC-S380 を動かした実例もあります。Sony の仕様書も macOS を動作対象に含めています。 ([nfcpy][3])

ただし、**いちばん再現性が高い実績は Linux 側** です。nfcpy の成功報告や記事は Raspberry Pi / Linux が多いです。なので、原因切り分けとしては **同じデバイス・同じフレームで Linux に持っていく** のが最短です。Linux で即成功し、macOS だけ `0x7F` なら、あなたの実装より **OS/libusb 差** を疑うべきです。逆に Linux でも `0x7F` なら、実装/手順の問題です。 ([Zenn][4])

### 5. 最初に成功する既知のコマンドシーケンス

公開されている成功例から、そのまま基準にできる最小系列はこれです。

1. `open`
2. `selectConfiguration(1)` / `set_active_configuration(1)`
3. `claimInterface(0)`
4. ACK 送信
   `00 00 FF 00 FF 00`
5. `SetCommandType 01`
   送信: `00 00 FF FF FF 03 00 FD D6 2A 01 FF 00`
   期待応答: ACK の後に `00 00 FF FF FF 03 00 FD D7 2B 00 FE 00`
6. `GetFirmwareVersion`
   送信: `00 00 FF FF FF 02 00 FE D6 20 0A 00`
   期待応答: ACK の後に `00 00 FF FF FF 04 00 FC D7 21 11 01 F6 00`
7. `GetPDDataVersion`
   送信: `00 00 FF FF FF 02 00 FE D6 22 08 00`
   期待応答: ACK の後に `00 00 FF FF FF 04 00 FC D7 23 00 01 05 00`
8. `SwitchRF 00`
   送信: `00 00 FF FF FF 03 00 FD D6 06 00 24 00`
   期待応答: ACK の後に `00 00 FF FF FF 03 00 FD D7 07 00 22 00` ([GitHub][1])

あなたの実装は、まずこの系列に **1バイトも違わず一致** させるべきです。
この段階では `GetFirmwareVersion` 単発で試すより、**必ず `SetCommandType 01` を先に置く** 方がよいです。公開実装もそうしています。 ([GitHub][1])

## ここまでを踏まえた、いちばん妥当な根本原因

私はこう見ます。

**第一候補**
`set_active_configuration(1)` を含む USB 初期化が nfcpy/既知成功例と一致していない。
その結果、Port-100 が期待モードに入らず、すべてのコマンドに `0x7F` を返している。 ([GitHub][1])

**第二候補**
`SetCommandType` を先頭に置く開始系列が正確に再現できていない。
少しでもフレームや順序が違うと、後続が全部 `0x7F` になる可能性が高いです。成功例は exact bytes が公開されています。 ([GitHub][1])

**第三候補**
macOS + libusb 特有の相性。
ありえますが、公開実績があるので「本質的に不可能」とは言えません。 ([Beatcraft][5])

## いま取るべき具体策

1. `set_active_configuration(1)` を明示する。
2. `detach_kernel_driver(0)` は macOS では本質ではないので、そこに期待しない。
3. 最初の系列を **ACK → SetCommandType 01 → GetFirmwareVersion** に固定し、送受信を成功例の bytes と比較する。
4. `SetCommandType` の期待成功応答を `D7 2B 00` 含みで厳密比較する。
5. 同じハードで **nfcpy をその Mac で実行** する。

   * nfcpy も `0x7F` なら、環境/OS/デバイス寄り
   * nfcpy は通るのに Rust は `0x7F` なら、実装差が確定
6. さらに切り分けるなら Linux で同じフレームを試す。 ([nfcpy][3])

必要なら次に、`rusb` で **既知成功系列をそのまま再現した最小 Rust サンプル** をこちらで組みます。

[1]: https://github.com/saturday06/webusb-felica/blob/gh-pages/demo.html "webusb-felica/demo.html at gh-pages · saturday06/webusb-felica · GitHub"
[2]: https://github.com/nfcpy/nfcpy/blob/master/src/nfc/clf/rcs956.py?utm_source=chatgpt.com "nfcpy/src/nfc/clf/rcs956.py at master"
[3]: https://nfcpy.readthedocs.io/en/latest/overview.html "Overview — nfcpy 1.0.4 documentation"
[4]: https://zenn.dev/ichidomisssuru/articles/8afc96c55803f5?utm_source=chatgpt.com "Raspberry PiでRS-C380を使うメモ"
[5]: https://www.beatcraft.com/labs/2021/12/macosnfc.html "macOSでNFC | labs"
