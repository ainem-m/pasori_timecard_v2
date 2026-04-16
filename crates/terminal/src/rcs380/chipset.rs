use super::frame;
use super::transport::{Transport, TransportError};
use pasori_core::port::reader::{CardId, CardScanned, ReaderBackend, ReaderError, ReaderStatus};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use thiserror::Error;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

#[derive(Debug, Error)]
pub enum ChipsetError {
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),
    #[error("frame error: {0}")]
    Frame(#[from] frame::FrameError),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("timeout")]
    Timeout,
}

pub struct Chipset<T: Transport> {
    transport: T,
}

impl<T: Transport> Chipset<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// チップセット初期化 (SetCommandType + SwitchRF + InSetRF + InSetProtocol)。
    pub fn initialize(&self) -> Result<(), ChipsetError> {
        // Step 1: SetCommandType (通信タイプ 3 = FeliCa)
        self.send_command_and_recv(&[0xD6, 0x2A, 0x01, 0x03])?;

        // Step 2: SwitchRF (RF ON)
        self.send_command_and_recv(&[0xD6, 0x06, 0x00])?;

        // Step 3: InSetRF (212F パラメータ)
        self.send_command_and_recv(&[0xD6, 0x00, 0x01, 0x01, 0x0F, 0x01])?;

        // Step 4: InSetProtocol (タイムアウト等)
        let protocol_cmd = [
            0xD6, 0x02, 0x00, 0x18, 0x01, 0x01, 0x18, 0x02, 0x07, 0x18, 0x03, 0x07, 0x18, 0x04,
            0x00, 0x18, 0x05, 0x00,
        ];
        self.send_command_and_recv(&protocol_cmd)?;

        Ok(())
    }

    /// FeliCa カードをポーリングし、IDm を取得する。
    /// カードが存在しない場合は Ok(None)。
    pub fn felica_polling(&self, system_code: u16) -> Result<Option<String>, ChipsetError> {
        let sc_hi = ((system_code >> 8) & 0xFF) as u8;
        let sc_lo = (system_code & 0xFF) as u8;

        let sensf_req = [0x06, 0x04, sc_hi, sc_lo, 0x01, 0x00];

        let mut cmd = vec![0xD6, 0x04, 0x6E, 0x00];
        cmd.extend_from_slice(&sensf_req);

        let response = self.send_command_and_recv(&cmd)?;

        // レスポンス構造: [D7, 05, STATUS, TIMEOUT, LEN, DATA...]
        if response.len() < 5 {
            return Err(ChipsetError::Protocol("response too short".to_string()));
        }

        let status = response[2];
        let len = response[4];

        if status != 0x00 || len == 0 {
            return Ok(None);
        }

        // DATA の構造: [LEN, 01, IDm(8bytes), PMm(8bytes), ...]
        if response.len() < 5 + 1 + 8 {
            return Ok(None);
        }

        let idm_start = 6; // skip [D7, 05, STATUS, TIMEOUT, LEN, 01] (first 6 bytes)
        let idm_bytes = &response[idm_start..idm_start + 8];

        let hex = idm_bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join("");

        Ok(Some(hex))
    }

    /// RF をオフにしてクリーンアップ。
    pub fn shutdown(&self) -> Result<(), ChipsetError> {
        self.send_command_and_recv(&[0xD6, 0x06, 0x00])?;
        Ok(())
    }

    /// コマンドを送信し、ACK と レスポンスを受信する。
    fn send_command_and_recv(&self, cmd: &[u8]) -> Result<Vec<u8>, ChipsetError> {
        let frame = frame::encode(cmd);
        self.transport.send(&frame)?;

        let mut ack_buf = vec![0u8; 6];
        let ack_size = self.transport.recv(&mut ack_buf, 1000)?;

        if ack_size == 6 && ack_buf == vec![0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00] {
            // ACK 受信
        } else {
            return Err(ChipsetError::Protocol(
                "expected ACK frame".to_string(),
            ));
        }

        let mut resp_buf = vec![0u8; 256];
        let resp_size = self.transport.recv(&mut resp_buf, 2500)?;
        resp_buf.truncate(resp_size);

        let payload = frame::decode(&resp_buf)?;

        match payload {
            Some(p) => {
                // ステータス検証: レスポンスの 3 番目バイト (index 2) が 0x00 なら成功
                if p.len() > 2 && p[2] != 0x00 {
                    return Err(ChipsetError::Protocol(format!(
                        "non-zero status: 0x{:02X}",
                        p[2]
                    )));
                }
                Ok(p)
            }
            None => Err(ChipsetError::Protocol(
                "expected data frame, got ACK".to_string(),
            )),
        }
    }
}

pub struct RCS380ReaderBackend {
    status: Arc<Mutex<ReaderStatus>>,
    tx: broadcast::Sender<CardScanned>,
    handle: Mutex<Option<JoinHandle<()>>>,
    cancel: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
}

impl RCS380ReaderBackend {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(16);
        Self {
            status: Arc::new(Mutex::new(ReaderStatus::Disconnected)),
            tx,
            handle: Mutex::new(None),
            cancel: Mutex::new(None),
        }
    }

    fn set_status(&self, s: ReaderStatus) {
        *self.status.lock().expect("status lock") = s;
    }
}

impl Default for RCS380ReaderBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ReaderBackend for RCS380ReaderBackend {
    async fn start(&self) -> Result<(), ReaderError> {
        let mut handle_guard = self.handle.lock().expect("handle lock");
        if handle_guard.is_some() {
            return Ok(());
        }

        self.set_status(ReaderStatus::Connecting);

        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        *self.cancel.lock().expect("cancel lock") = Some(cancel_tx);

        let tx = self.tx.clone();
        let status = self.status.clone();

        let join = tokio::task::spawn_blocking(move || {
            poll_loop(tx, status, cancel_rx);
        });

        *handle_guard = Some(join);
        Ok(())
    }

    async fn stop(&self) -> Result<(), ReaderError> {
        if let Some(tx) = self.cancel.lock().expect("cancel lock").take() {
            let _ = tx.send(true);
        }
        let handle = self.handle.lock().expect("handle lock").take();
        if let Some(h) = handle {
            let _ = h.await;
        }
        self.set_status(ReaderStatus::Disconnected);
        Ok(())
    }

    fn status(&self) -> ReaderStatus {
        self.status.lock().expect("status lock").clone()
    }

    fn subscribe(&self) -> broadcast::Receiver<CardScanned> {
        self.tx.subscribe()
    }
}

fn poll_loop(
    tx: broadcast::Sender<CardScanned>,
    status: Arc<Mutex<ReaderStatus>>,
    cancel: tokio::sync::watch::Receiver<bool>,
) {
    let transport = match super::transport::UsbTransport::open() {
        Ok(t) => t,
        Err(e) => {
            *status.lock().expect("status lock") = ReaderStatus::Error(format!("USB open error: {e}"));
            return;
        }
    };

    let chipset = Chipset::new(transport);

    if let Err(e) = chipset.initialize() {
        *status.lock().expect("status lock") = ReaderStatus::Error(format!("initialize error: {e}"));
        return;
    }

    *status.lock().expect("status lock") = ReaderStatus::Ready;

    let mut last_seen: HashMap<String, std::time::Instant> = HashMap::new();
    let mut use_transport_system_code = false;
    const SUPPRESSION_WINDOW_SECS: u64 = 5;

    loop {
        if *cancel.borrow() {
            break;
        }

        let system_code = if use_transport_system_code { 0x0003 } else { 0xFFFF };
        use_transport_system_code = !use_transport_system_code;

        match chipset.felica_polling(system_code) {
            Ok(Some(idm_hex)) => {
                let now_instant = std::time::Instant::now();
                let suppress = last_seen
                    .get(&idm_hex)
                    .map(|t| now_instant.duration_since(*t).as_secs() < SUPPRESSION_WINDOW_SECS)
                    .unwrap_or(false);

                if !suppress {
                    last_seen.insert(idm_hex.clone(), now_instant);
                    let scanned_at = tokyo_now();
                    tracing::info!(card_id = %idm_hex, "card scanned");
                    let _ = tx.send(CardScanned {
                        card_id: CardId(idm_hex),
                        scanned_at,
                    });
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::debug!("polling error: {e}");
            }
        }

        std::thread::sleep(Duration::from_millis(200));
    }

    let _ = chipset.shutdown();
}

fn tokyo_now() -> jiff::Zoned {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::get("Asia/Tokyo").expect("Asia/Tokyo tz"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rcs380::transport::MockTransport;

    #[test]
    // 初期状態は Disconnected である。
    fn initial_status_is_disconnected() {
        let reader = RCS380ReaderBackend::new();
        assert_eq!(reader.status(), ReaderStatus::Disconnected);
    }

    #[test]
    // subscribe は broadcast の Receiver を返す。
    fn subscribe_returns_receiver() {
        let reader = RCS380ReaderBackend::new();
        let _rx = reader.subscribe();
    }

    #[test]
    #[ignore = "実機テスト: RC-S380を接続した状態で手動実行"]
    // RC-S380との実機接続テスト: 初期化～ポーリング～シャットダウン
    fn hardware_full_cycle() {
        use super::super::transport::UsbTransport;

        let transport = match UsbTransport::open() {
            Ok(t) => {
                println!("✓ RC-S380接続確認");
                t
            }
            Err(e) => {
                panic!("RC-S380が見つかりません: {:?}", e);
            }
        };

        let chipset = Chipset::new(transport);

        match chipset.initialize() {
            Ok(()) => println!("✓ チップセット初期化成功"),
            Err(e) => panic!("初期化失敗: {:?}", e),
        }

        println!("カードをタッチしてください...");
        for system_code in &[0xFFFF, 0x0003] {
            match chipset.felica_polling(*system_code) {
                Ok(Some(idm)) => {
                    println!("✓ カード検出: IDm={}", idm);
                    return;
                }
                Ok(None) => {
                    println!("  システムコード 0x{:04X}: カードなし", system_code);
                }
                Err(e) => {
                    println!("  ポーリング error: {:?}", e);
                }
            }
        }

        match chipset.shutdown() {
            Ok(()) => println!("✓ シャットダウン成功"),
            Err(e) => panic!("シャットダウン失敗: {:?}", e),
        }
    }

    #[test]
    // Chipset::initialize に MockTransport で正常レスポンスをセット
    fn chipset_initialize_success() {
        let transport = MockTransport::new();

        // 4 つのコマンドに対して ACK + レスポンスを返す
        // レスポンス: [D7, 2B, 00] (SetCommandType response)
        let resp = frame::encode(&[0xD7, 0x2B, 0x00]);

        for _ in 0..4 {
            transport.queue_response(vec![0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00]); // ACK
            transport.queue_response(resp.clone()); // フレーム形式のレスポンス
        }

        let chipset = Chipset::new(transport);
        let result = chipset.initialize();
        assert!(result.is_ok());
    }

    #[test]
    // Chipset::felica_polling でカード検出
    fn chipset_felica_polling_card_detected() {
        let transport = MockTransport::new();

        transport.queue_response(vec![0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00]); // ACK

        // レスポンス payload: [D7, 05, 00, TIMEOUT, LEN, 01, IDm(8), PMm(8)]
        // IDm = [01, FE, 01, 00, 11, 22, 33, 44]
        let payload = vec![
            0xD7, 0x05, 0x00, 0x00, 0x12, 0x01,  // header
            0x01, 0xFE, 0x01, 0x00, 0x11, 0x22, 0x33, 0x44,  // IDm (8 bytes)
            0x55, 0x66, 0x77, 0x88,  // PMm (8 bytes)
        ];
        let frame_response = frame::encode(&payload);
        transport.queue_response(frame_response);

        let chipset = Chipset::new(transport);
        let result = chipset.felica_polling(0xFFFF);
        assert!(result.is_ok());
        let idm = result.unwrap();
        assert_eq!(idm, Some("01FE010011223344".to_string()));
    }

    #[test]
    // Chipset::felica_polling でカード未検出
    fn chipset_felica_polling_no_card() {
        let transport = MockTransport::new();

        transport.queue_response(vec![0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00]); // ACK

        // レスポンス payload: [D7, 05, 00, TIMEOUT, 00] (len=0 = no card)
        let payload = vec![0xD7, 0x05, 0x00, 0x00, 0x00];
        let frame_response = frame::encode(&payload);
        transport.queue_response(frame_response);

        let chipset = Chipset::new(transport);
        let result = chipset.felica_polling(0xFFFF);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
    }
}
