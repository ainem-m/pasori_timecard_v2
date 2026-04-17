use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("usb error: {0}")]
    Usb(String),
    #[error("timeout")]
    Timeout,
    #[error("device not found")]
    DeviceNotFound,
}

pub trait Transport: Send + Sync {
    /// フレームエンコード済みデータを送信する。
    fn send(&self, data: &[u8]) -> Result<(), TransportError>;

    /// 受信バッファにデータを読み取る。
    /// 戻り値は読み取ったバイト数。
    fn recv(&self, buf: &mut [u8], timeout_ms: u64) -> Result<usize, TransportError>;
}

#[cfg(test)]
pub struct MockTransport {
    responses: std::sync::Mutex<std::collections::VecDeque<Vec<u8>>>,
    sent: std::sync::Mutex<Vec<Vec<u8>>>,
}

#[cfg(test)]
impl MockTransport {
    pub fn new() -> Self {
        Self {
            responses: std::sync::Mutex::new(std::collections::VecDeque::new()),
            sent: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn queue_response(&self, data: Vec<u8>) {
        self.responses.lock().unwrap().push_back(data);
    }
}

#[cfg(test)]
impl Transport for MockTransport {
    fn send(&self, data: &[u8]) -> Result<(), TransportError> {
        self.sent.lock().unwrap().push(data.to_vec());
        Ok(())
    }

    fn recv(&self, buf: &mut [u8], _timeout_ms: u64) -> Result<usize, TransportError> {
        let mut responses = self.responses.lock().unwrap();
        if let Some(data) = responses.pop_front() {
            let len = data.len().min(buf.len());
            buf[..len].copy_from_slice(&data[..len]);
            Ok(len)
        } else {
            Err(TransportError::Timeout)
        }
    }
}

pub struct UsbTransport {
    handle: rusb::DeviceHandle<rusb::GlobalContext>,
}

const RC_S380_VID: u16 = 0x054C;
const RC_S380_PID: u16 = 0x06C3;

impl UsbTransport {
    /// RC-S380 を USB から検索して接続する。
    /// nfcpy と同じシーケンス: open → set_configuration(1) → claim → ACK soft-reset
    /// USB reset は行わない（re-enumeration 後にデバイスが invalid state になるため）
    pub fn open() -> Result<Self, TransportError> {
        let devices = rusb::devices().map_err(|e| TransportError::Usb(format!("{:?}", e)))?;

        for device in devices.iter() {
            let desc = match device.device_descriptor() {
                Ok(d) => d,
                Err(_) => continue,
            };

            if desc.vendor_id() == RC_S380_VID && desc.product_id() == RC_S380_PID {
                let handle = device
                    .open()
                    .map_err(|e| TransportError::Usb(format!("{:?}", e)))?;

                // macOS では no-op だが Linux では必要な場合がある
                let _ = handle.detach_kernel_driver(0);

                // nfcpy の set_configuration() 相当: エンドポイント状態をクリアする
                // 既に config 1 の場合でも SET_CONFIGURATION 制御転送でエンドポイントをリセット
                // エラーは無視（既に claim 済みの場合は BUSY が返ることがある）
                let _ = handle.set_active_configuration(1);

                handle
                    .claim_interface(0)
                    .map_err(|e| TransportError::Usb(format!("{:?}", e)))?;

                // nfcpy 互換のソフトリセット: ACK フレームを送信してデバイスステートをクリア
                let ack = [0x00u8, 0x00, 0xFF, 0x00, 0xFF, 0x00];
                let _ = handle.write_bulk(0x02, &ack, Duration::from_millis(100));
                std::thread::sleep(Duration::from_millis(10));
                // ACK への応答をドレイン
                let mut drain_buf = vec![0u8; 256];
                while handle
                    .read_bulk(0x81, &mut drain_buf, Duration::from_millis(50))
                    .is_ok()
                {}

                return Ok(UsbTransport { handle });
            }
        }

        Err(TransportError::DeviceNotFound)
    }
}

impl Transport for UsbTransport {
    fn send(&self, data: &[u8]) -> Result<(), TransportError> {
        self.handle
            .write_bulk(0x02, data, Duration::from_millis(1000))
            .map(|_| ())
            .map_err(|e| TransportError::Usb(format!("{:?}", e)))
    }

    fn recv(&self, buf: &mut [u8], timeout_ms: u64) -> Result<usize, TransportError> {
        self.handle
            .read_bulk(0x81, buf, Duration::from_millis(timeout_ms))
            .map_err(|e| {
                // rusb::Error::Timeout for timeout errors
                if let rusb::Error::Timeout = e {
                    TransportError::Timeout
                } else {
                    TransportError::Usb(format!("{:?}", e))
                }
            })
    }
}

impl Drop for UsbTransport {
    fn drop(&mut self) {
        let _ = self.handle.release_interface(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "実機テスト: RC-S380接続時に手動実行"]
    // RC-S380 の USB インターフェース・エンドポイント構成を表示する診断テスト
    fn hardware_usb_descriptor_dump() {
        let devices = rusb::devices().expect("usb devices");
        for device in devices.iter() {
            let desc = match device.device_descriptor() {
                Ok(d) => d,
                Err(_) => continue,
            };
            if desc.vendor_id() != 0x054C || desc.product_id() != 0x06C3 {
                continue;
            }
            println!(
                "RC-S380 found: Bus {:03} Device {:03}",
                device.bus_number(),
                device.address()
            );
            let config = device
                .active_config_descriptor()
                .expect("config descriptor");
            println!("  Configuration: {}", config.number());
            for iface in config.interfaces() {
                for setting in iface.descriptors() {
                    println!(
                        "  Interface {} (alt={}) class={:02X} subclass={:02X} proto={:02X}",
                        setting.interface_number(),
                        setting.setting_number(),
                        setting.class_code(),
                        setting.sub_class_code(),
                        setting.protocol_code(),
                    );
                    for ep in setting.endpoint_descriptors() {
                        println!(
                            "    Endpoint 0x{:02X} dir={:?} type={:?}",
                            ep.address(),
                            ep.direction(),
                            ep.transfer_type(),
                        );
                    }
                }
            }
            return;
        }
        panic!("RC-S380 not found");
    }

    fn raw_exchange(handle: &rusb::DeviceHandle<rusb::GlobalContext>, name: &str, payload: &[u8]) {
        let cmd = super::super::frame::encode(payload);
        println!("\n[{}] 送信: {:02X?}", name, cmd);
        match handle.write_bulk(0x02, &cmd, Duration::from_millis(1000)) {
            Ok(_) => {}
            Err(e) => {
                println!("  送信エラー: {:?}", e);
                return;
            }
        }
        for i in 1..=3 {
            let mut buf = vec![0u8; 256];
            match handle.read_bulk(0x81, &mut buf, Duration::from_millis(1000)) {
                Ok(n) => {
                    println!("  受信{} ({} bytes): {:02X?}", i, n, &buf[..n]);
                    let decoded = super::super::frame::decode(&buf[..n]);
                    println!("  decode: {:?}", decoded);
                }
                Err(e) => {
                    println!("  受信{} エラー: {:?}", i, e);
                    break;
                }
            }
        }
    }

    #[test]
    #[ignore = "実機テスト: RC-S380接続時に手動実行"]
    // nfcpy 初期化シーケンス全体を生バイトレベルで診断する
    fn hardware_nfcpy_init_sequence() {
        let usb_transport = UsbTransport::open().expect("USB open");
        let handle = &usb_transport.handle;

        println!("=== ACK ソフトリセット ===");
        let ack = [0x00u8, 0x00, 0xFF, 0x00, 0xFF, 0x00];
        println!("ACK 送信: {:02X?}", ack);
        let _ = handle.write_bulk(0x02, &ack, Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(10));
        let mut drain = vec![0u8; 256];
        while let Ok(n) = handle.read_bulk(0x81, &mut drain, Duration::from_millis(50)) {
            println!("  ドレイン: {:02X?}", &drain[..n]);
        }

        println!("\n=== 各コマンド試行 ===");
        // SetCommandType(1) - nfcpy 互換 3 bytes のみ
        raw_exchange(handle, "SetCommandType(1)", &[0xD6, 0x2A, 0x01]);
        // GetFirmwareVersion
        raw_exchange(handle, "GetFirmwareVersion", &[0xD6, 0x20]);
        // SwitchRF(off)
        raw_exchange(handle, "SwitchRF(off)", &[0xD6, 0x06, 0x00]);
    }

    #[test]
    #[ignore = "実機テスト: RC-S380接続時に手動実行"]
    // GetFirmwareVersion の生バイト送受信を確認する診断テスト（複数受信付き）
    fn hardware_raw_get_firmware_version() {
        let transport = UsbTransport::open().expect("USB open");

        // コマンド送信前にドレイン（前のセッションの残留データを除去）
        {
            let handle = &transport.handle;
            let mut drain = vec![0u8; 256];
            let mut drain_count = 0;
            while let Ok(n) = handle.read_bulk(0x81, &mut drain, Duration::from_millis(100)) {
                println!("ドレイン: {} bytes: {:02X?}", n, &drain[..n]);
                drain_count += 1;
                if drain_count > 10 {
                    break;
                }
            }
            println!("ドレイン完了 ({} packets)", drain_count);
        }

        // GetFirmwareVersion: [D6, 20] を拡張フレームで送信
        let cmd = super::super::frame::encode(&[0xD6, 0x20]);
        println!("送信: {:02X?}", cmd);
        transport.send(&cmd).expect("send");

        // 最大5回受信して全パケットを表示
        for i in 1..=5 {
            let mut buf = vec![0u8; 256];
            match transport.recv(&mut buf, 1000) {
                Ok(n) => {
                    println!("受信{} ({} bytes): {:02X?}", i, n, &buf[..n]);
                    let frame = super::super::frame::decode(&buf[..n]);
                    println!("  decode: {:?}", frame);
                }
                Err(e) => {
                    println!("受信{} エラー: {:?}", i, e);
                    break;
                }
            }
        }
    }

    #[test]
    #[ignore = "実機テスト: RC-S380接続時に手動実行"]
    // USB接続のみのテスト (SetCommandType コマンド送信)
    fn hardware_usb_communication() {
        let transport = match UsbTransport::open() {
            Ok(t) => {
                println!("✓ RC-S380 USB接続確認");
                t
            }
            Err(e) => panic!("USB接続失敗: {:?}", e),
        };

        // SetCommandType コマンド: [D6, 2A, 01, 03]
        let cmd = super::super::frame::encode(&[0xD6, 0x2A, 0x01, 0x03]);
        println!("→ SetCommandType送信: {:?}", cmd);

        match transport.send(&cmd) {
            Ok(()) => println!("✓ コマンド送信成功"),
            Err(e) => panic!("送信失敗: {:?}", e),
        }

        // ACK 受信
        let mut ack_buf = vec![0u8; 6];
        match transport.recv(&mut ack_buf, 1000) {
            Ok(n) => {
                println!("✓ ACK受信: {} bytes = {:?}", n, &ack_buf[..n]);
            }
            Err(e) => panic!("ACK受信失敗: {:?}", e),
        }

        // レスポンス受信 (複数回試行)
        let mut all_data = Vec::new();
        for attempt in 0..3 {
            let mut resp_buf = vec![0u8; 256];
            match transport.recv(&mut resp_buf, 1000) {
                Ok(n) => {
                    println!("  試行{}: {} bytes受信", attempt + 1, n);
                    println!("    hex: {:02X?}", &resp_buf[..n]);
                    all_data.extend_from_slice(&resp_buf[..n]);
                }
                Err(TransportError::Timeout) => {
                    println!("  試行{}: タイムアウト", attempt + 1);
                    break;
                }
                Err(e) => {
                    println!("  試行{}: エラー {:?}", attempt + 1, e);
                    break;
                }
            }
        }

        println!("✓ 受信合計: {} bytes", all_data.len());
        println!("  完全hex: {:02X?}", all_data);

        match super::super::frame::decode(&all_data) {
            Ok(super::super::frame::DecodedFrame::Data(payload)) => {
                println!("  ✓ デコード成功");
                println!("    payload: {:02X?}", payload);
            }
            Ok(super::super::frame::DecodedFrame::Ack) => println!("  (ACK frame)"),
            Ok(super::super::frame::DecodedFrame::Error) => println!("  (Error frame)"),
            Err(e) => println!("  ✗ デコードエラー: {:?}", e),
        }
    }
}
