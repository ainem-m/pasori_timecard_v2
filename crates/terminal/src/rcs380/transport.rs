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

    pub fn sent_data(&self) -> Vec<Vec<u8>> {
        self.sent.lock().unwrap().clone()
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

impl UsbTransport {
    /// RC-S380 を USB から検索して接続する。
    pub fn open() -> Result<Self, TransportError> {
        let devices = rusb::devices().map_err(|e| TransportError::Usb(format!("{:?}", e)))?;

        for device in devices.iter() {
            let desc = match device.device_descriptor() {
                Ok(d) => d,
                Err(_) => continue,
            };

            if desc.vendor_id() == 0x054C && desc.product_id() == 0x06C3 {
                let handle = device.open().map_err(|e| TransportError::Usb(format!("{:?}", e)))?;

                // カーネルドライバがアタッチされていれば detach する
                let _ = handle.detach_kernel_driver(0);

                handle.claim_interface(0).map_err(|e| TransportError::Usb(format!("{:?}", e)))?;

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
            Ok(Some(payload)) => {
                println!("  ✓ デコード成功");
                println!("    payload: {:02X?}", payload);
            }
            Ok(None) => println!("  (ACK frame)"),
            Err(e) => println!("  ✗ デコードエラー: {:?}", e),
        }
    }
}
