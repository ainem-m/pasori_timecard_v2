use super::frame::{self, DecodedFrame};
use super::transport::{Transport, TransportError};
use pasori_core::port::reader::{CardId, CardScanned, ReaderBackend, ReaderError, ReaderStatus};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
    #[error("device rejected command (application error frame)")]
    DeviceRejected,
}

pub struct Chipset<T: Transport> {
    transport: T,
}

impl<T: Transport> Chipset<T> {
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// ファームウェアバージョンを取得する（疎通確認を兼ねる）。
    pub fn get_firmware_version(&self) -> Result<String, ChipsetError> {
        let response = self.send_command_and_recv(&[0xD6, 0x20])?;
        // response: [D7, 21, ver_minor, ver_major]
        if response.len() < 4 {
            return Err(ChipsetError::Protocol(
                "get_firmware_version response too short".to_string(),
            ));
        }
        let minor = response[2];
        let major = response[3];
        Ok(format!("{}.{:02X}", major, minor))
    }

    /// チップセット初期化 (nfcpy Chipset.__init__ + sense_ttf 互換)
    pub fn initialize(&self) -> Result<(), ChipsetError> {
        // Step 1: SetCommandType(1) — Port-100 拡張コマンドモードを有効化
        tracing::debug!("Step 1: SetCommandType(1)");
        self.send_command_and_recv(&[0xD6, 0x2A, 0x01])?;

        // Step 2: GetFirmwareVersion
        tracing::debug!("Step 2: GetFirmwareVersion");
        let ver = self.get_firmware_version()?;
        tracing::info!(fw = %ver, "RC-S380 firmware version");

        // Step 3: GetPDDataVersion (nfcpy Chipset.__init__ で必須)
        tracing::debug!("Step 3: GetPDDataVersion");
        self.send_command_and_recv(&[0xD6, 0x22])?;

        // Step 4: SwitchRF off
        tracing::debug!("Step 4: SwitchRF off");
        self.send_command_and_recv(&[0xD6, 0x06, 0x00])?;

        // Step 5: InSetRF (212F: send_set=1, comm_type=0x01, recv_set=0x0F, recv_comm_type=0x01)
        tracing::debug!("Step 5: InSetRF");
        self.send_command_and_recv(&[0xD6, 0x00, 0x01, 0x01, 0x0F, 0x01])?;

        // Step 6: InSetProtocol (nfcpy in_set_protocol_defaults — tag-value ペア, END マーカーなし)
        tracing::debug!("Step 6: InSetProtocol");
        #[rustfmt::skip]
        let defaults_payload = [
            0x00u8, 0x18,  // INITIAL_GUARD_TIME = 24 (0x18)
            0x01, 0x01,    // ADD_CRC = 1
            0x02, 0x01,    // CHECK_CRC = 1
            0x03, 0x00,    // MULTI_CARD = 0
            0x04, 0x00,    // ADD_PARITY = 0
            0x05, 0x00,    // CHECK_PARITY = 0
            0x06, 0x00,    // BITWISE_ANTICOLL = 0
            0x07, 0x08,    // LAST_BYTE_BIT_COUNT = 8
            0x08, 0x00,    // MIFARE_CRYPTO = 0
            0x09, 0x00,    // ADD_SOF = 0
            0x0A, 0x00,    // CHECK_SOF = 0
            0x0B, 0x00,    // ADD_EOF = 0
            0x0C, 0x00,    // CHECK_EOF = 0
            0x0E, 0x04,    // DEAF_TIME = 4
            0x10, 0x00,    // CRM_MIN_LEN = 0
            0x11, 0x00,    // T1_TAG_RRDD = 0
            0x12, 0x00,    // RFCA = 0
            0x13, 0x06,    // GUARD_TIME = 6
        ];
        let mut protocol_cmd = vec![0xD6u8, 0x02];
        protocol_cmd.extend_from_slice(&defaults_payload);
        self.send_command_and_recv(&protocol_cmd)?;

        Ok(())
    }

    /// FeliCa カードをポーリングし、IDm を取得する。
    /// カードが存在しない場合は Ok(None)。
    /// nfcpy sense_ttf と同じシーケンス: InSetRF → InSetProtocol → InCommRF
    ///
    /// `timeout_raw`: InCommRF タイムアウト値 (デバイス単位, ~1ms/unit)
    pub fn felica_polling(
        &self,
        system_code: u16,
        timeout_raw: u16,
    ) -> Result<Option<String>, ChipsetError> {
        // nfcpy sense_ttf: ポーリングごとに InSetRF + InSetProtocol を送る
        self.send_command_and_recv(&[0xD6, 0x00, 0x01, 0x01, 0x0F, 0x01])?; // InSetRF(212F)

        #[rustfmt::skip]
        let defaults_payload = [
            0x00u8, 0x18, 0x01, 0x01, 0x02, 0x01, 0x03, 0x00,
            0x04, 0x00, 0x05, 0x00, 0x06, 0x00, 0x07, 0x08,
            0x08, 0x00, 0x09, 0x00, 0x0A, 0x00, 0x0B, 0x00,
            0x0C, 0x00, 0x0E, 0x04, 0x0F, 0x00, 0x10, 0x00,
            0x11, 0x00, 0x12, 0x00, 0x13, 0x06,
        ];
        let mut proto = vec![0xD6u8, 0x02];
        proto.extend_from_slice(&defaults_payload);
        self.send_command_and_recv(&proto)?; // InSetProtocol(defaults)

        // nfcpy: in_set_protocol(initial_guard_time=24) — 2回目
        self.send_command_and_recv(&[0xD6, 0x02, 0x00, 0x18])?;

        let sc_hi = ((system_code >> 8) & 0xFF) as u8;
        let sc_lo = (system_code & 0xFF) as u8;
        let t_lo = (timeout_raw & 0xFF) as u8;
        let t_hi = ((timeout_raw >> 8) & 0xFF) as u8;

        // nfcpy default sensf_req: [len=6, 0x00, SC_HI, SC_LO, RC=0x01, TS=0x00]
        // byte 1 is 0x00 (not 0x04) — matches nfcpy's bytearray.fromhex("00FFFF0100")
        let sensf_req = [0x06, 0x00, sc_hi, sc_lo, 0x01, 0x00];

        let mut cmd = vec![0xD6, 0x04, t_lo, t_hi];
        cmd.extend_from_slice(&sensf_req);

        let response = self.send_command_and_recv(&cmd)?;

        // InCommRF レスポンス構造 (nfcpy 準拠):
        // [D7, 05, S1, S2, S3, S4, rec_no, frame_data...]
        //  0    1   2   3   4   5    6       7+
        // S1-S4: ステータス (全 0x00 = 成功)
        // rec_no: 受信フレーム数/バイト数
        // frame_data: SensF レスポンスフレーム [frame_len, 0x01, IDm(8), PMm(8), ...]
        if response.len() < 6 {
            return Err(ChipsetError::Protocol("response too short".to_string()));
        }

        // S1 != 0x00 はタイムアウト/カードなし (0x80 = timeout)
        if response[2] != 0x00 {
            return Ok(None);
        }

        // rec_no == 0 はカードなし
        if response.len() < 7 || response[6] == 0 {
            return Ok(None);
        }

        // SensF レスポンスフレーム: response[7]=frame_len, response[8]=0x01, response[9..17]=IDm
        if response.len() < 9 + 8 {
            return Ok(None);
        }

        let idm_start = 9; // D7(0) 05(1) S1(2) S2(3) S3(4) S4(5) rec_no(6) frame_len(7) resp_code(8)
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
        tracing::trace!(cmd = ?cmd, "sending USB frame");
        self.transport.send(&frame)?;

        tracing::trace!("waiting for ACK...");
        let mut ack_buf = [0u8; 512];
        let ack_size = self.transport.recv(&mut ack_buf, 1000)?;

        let is_ack = ack_size == 6 && ack_buf[..6] == [0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00];
        if !is_ack {
            tracing::debug!(size = ack_size, hex = ?&ack_buf[..ack_size], "received non-ACK instead of ACK");
            // 6バイトACKではなかった場合、これ自体がレスポンスの可能性もある
            let data = ack_buf[..ack_size].to_vec();
            return match frame::decode(&data)? {
                DecodedFrame::Data(p) => Ok(p),
                DecodedFrame::Ack => Err(ChipsetError::Protocol(
                    "unexpected ACK as first recv".to_string(),
                )),
                DecodedFrame::Error => {
                    tracing::warn!(cmd = ?cmd, "device rejected (error frame in recv1)");
                    Err(ChipsetError::DeviceRejected)
                }
            };
        }
        tracing::trace!("ACK received");

        tracing::trace!("waiting for response data...");
        let mut resp_buf = [0u8; 512];
        let resp_size = self.transport.recv(&mut resp_buf, 2500)?;
        let data = resp_buf[..resp_size].to_vec();
        tracing::trace!(size = resp_size, "response data received");

        match frame::decode(&data)? {
            DecodedFrame::Data(p) => Ok(p),
            DecodedFrame::Ack => Err(ChipsetError::Protocol(
                "expected data frame, got ACK".to_string(),
            )),
            DecodedFrame::Error => {
                tracing::warn!(
                    "device returned application error frame for cmd: {:02X?}",
                    cmd
                );
                Err(ChipsetError::DeviceRejected)
            }
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
        if let Err(error) = set_shared_status(&self.status, s) {
            tracing::error!(%error, "failed to update RCS380 reader status");
        }
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
        let mut handle_guard = self
            .handle
            .lock()
            .map_err(|_| ReaderError::Other("handle lock poisoned".to_string()))?;
        if handle_guard.is_some() {
            return Ok(());
        }

        self.set_status(ReaderStatus::Connecting);

        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        *self
            .cancel
            .lock()
            .map_err(|_| ReaderError::Other("cancel lock poisoned".to_string()))? = Some(cancel_tx);

        let tx = self.tx.clone();
        let status = self.status.clone();

        let join = tokio::task::spawn_blocking(move || {
            poll_loop(tx, status, cancel_rx);
        });

        *handle_guard = Some(join);
        Ok(())
    }

    async fn stop(&self) -> Result<(), ReaderError> {
        if let Some(tx) = self
            .cancel
            .lock()
            .map_err(|_| ReaderError::Other("cancel lock poisoned".to_string()))?
            .take()
        {
            let _ = tx.send(true);
        }
        let handle = self
            .handle
            .lock()
            .map_err(|_| ReaderError::Other("handle lock poisoned".to_string()))?
            .take();
        if let Some(h) = handle {
            let _ = h.await;
        }
        self.set_status(ReaderStatus::Disconnected);
        Ok(())
    }

    fn status(&self) -> ReaderStatus {
        shared_status_snapshot(&self.status)
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
            let _ = set_shared_status(&status, ReaderStatus::Error(format!("USB open error: {e}")));
            return;
        }
    };

    let chipset = Chipset::new(transport);

    if let Err(e) = chipset.initialize() {
        let _ = set_shared_status(
            &status,
            ReaderStatus::Error(format!("initialize error: {e}")),
        );
        return;
    }

    let _ = set_shared_status(&status, ReaderStatus::Ready);

    let mut last_seen: HashMap<String, std::time::Instant> = HashMap::new();
    let mut poll_counter: u32 = 0;
    const SUPPRESSION_WINDOW_SECS: u64 = 5;
    // iPhone エクスプレスカード対応:
    // 0xFFFF (全カード) は 10 回に 1 回だけ使い、残り 9 回は 0x0003 (交通系) のみ待受する。
    // 0xFFFF でポーリングすると iPhone がロック解除を要求するが、
    // 0x0003 でポーリングすればエクスプレスカードはロック不要で応答する。
    const WILDCARD_INTERVAL: u32 = 10;
    const TIMEOUT_SHORT_MS: u16 = 100; // 0xFFFF ポーリング用 (物理カード用、短め)
    const TIMEOUT_LONG_MS: u16 = 900; // 0x0003 ポーリング用 (iPhone エクスプレス用、長め)

    loop {
        if *cancel.borrow() {
            break;
        }

        let (system_code, timeout_raw) = if poll_counter % WILDCARD_INTERVAL == 0 {
            (0xFFFF_u16, TIMEOUT_SHORT_MS)
        } else {
            (0x0003_u16, TIMEOUT_LONG_MS)
        };
        poll_counter = poll_counter.wrapping_add(1);

        match chipset.felica_polling(system_code, timeout_raw) {
            Ok(Some(idm_hex)) => {
                let now_instant = std::time::Instant::now();
                let suppress = last_seen
                    .get(&idm_hex)
                    .map(|t| now_instant.duration_since(*t).as_secs() < SUPPRESSION_WINDOW_SECS)
                    .unwrap_or(false);

                if !suppress {
                    last_seen.insert(idm_hex.clone(), now_instant);
                    let scanned_at = match tokyo_now() {
                        Ok(scanned_at) => scanned_at,
                        Err(error) => {
                            let _ =
                                set_shared_status(&status, ReaderStatus::Error(error.to_string()));
                            tracing::error!(%error, "failed to get Tokyo time for card scan");
                            return;
                        }
                    };
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
    }

    let _ = chipset.shutdown();
}

fn tokyo_now() -> Result<jiff::Zoned, ReaderError> {
    let timezone = jiff::tz::TimeZone::get("Asia/Tokyo")
        .map_err(|error| ReaderError::Other(format!("Asia/Tokyo timezone unavailable: {error}")))?;
    Ok(jiff::Timestamp::now().to_zoned(timezone))
}

fn set_shared_status(status: &Mutex<ReaderStatus>, next: ReaderStatus) -> Result<(), ReaderError> {
    let mut guard = status
        .lock()
        .map_err(|_| ReaderError::Other("status lock poisoned".to_string()))?;
    *guard = next;
    Ok(())
}

fn shared_status_snapshot(status: &Mutex<ReaderStatus>) -> ReaderStatus {
    match status.lock() {
        Ok(guard) => guard.clone(),
        Err(_) => ReaderStatus::Error("status lock poisoned".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rcs380::transport::MockTransport;

    fn poison_mutex<T>(mutex: &Mutex<T>) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = mutex.lock().expect("test mutex lock should succeed");
            panic!("poison mutex for test");
        }));
    }

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
    // status mutex が poison されても panic せず Error を返す。
    fn status_returns_error_when_status_mutex_is_poisoned() {
        let reader = RCS380ReaderBackend::new();
        poison_mutex(&reader.status);

        let status = reader.status();

        assert_eq!(
            status,
            ReaderStatus::Error("status lock poisoned".to_string())
        );
    }

    fn queue_command_response(transport: &MockTransport, response_payload: &[u8]) {
        transport.queue_response(vec![0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00]); // ACK
        transport.queue_response(frame::encode(response_payload)); // 拡張フレーム形式のレスポンス
    }

    #[test]
    // get_firmware_version が [D7, 21, 0A, 01] を "1.0A" として返す。
    fn get_firmware_version_returns_version_string() {
        let transport = MockTransport::new();
        queue_command_response(&transport, &[0xD7, 0x21, 0x0A, 0x01]);

        let chipset = Chipset::new(transport);
        let result = chipset.get_firmware_version();
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
        assert_eq!(result.unwrap(), "1.0A");
    }

    #[test]
    // initialize は SetCommandType が失敗すると全体が失敗する。
    fn initialize_fails_when_set_command_type_fails() {
        let transport = MockTransport::new();
        // SetCommandType に対して Error Frame を返す
        transport.queue_response(vec![0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00]); // ACK
        transport.queue_response(vec![0x00, 0x00, 0xFF, 0x01, 0xFF, 0x7F, 0x81, 0x00]); // Error Frame

        let chipset = Chipset::new(transport);
        let result = chipset.initialize();
        assert!(matches!(result, Err(ChipsetError::DeviceRejected)));
    }

    #[test]
    // デバイスが Error Frame を返したら DeviceRejected として扱う。
    fn send_command_returns_device_rejected_for_error_frame() {
        let transport = MockTransport::new();
        transport.queue_response(vec![0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00]); // ACK
        transport.queue_response(vec![0x00, 0x00, 0xFF, 0x01, 0xFF, 0x7F, 0x81, 0x00]); // Error Frame

        let chipset = Chipset::new(transport);
        // get_firmware_version を使って Error Frame を引き起こす
        let result = chipset.get_firmware_version();
        assert!(matches!(result, Err(ChipsetError::DeviceRejected)));
    }

    #[test]
    // Chipset::initialize に MockTransport で正常レスポンスをセット (nfcpy 互換 6 コマンド)
    fn chipset_initialize_success() {
        let transport = MockTransport::new();

        // Step1: SetCommandType(1)
        queue_command_response(&transport, &[0xD7, 0x2B, 0x00]);
        // Step2: GetFirmwareVersion
        queue_command_response(&transport, &[0xD7, 0x21, 0x0A, 0x01]);
        // Step3: GetPDDataVersion
        queue_command_response(&transport, &[0xD7, 0x23, 0x00, 0x01]);
        // Step4: SwitchRF
        queue_command_response(&transport, &[0xD7, 0x07, 0x00]);
        // Step5: InSetRF
        queue_command_response(&transport, &[0xD7, 0x01, 0x00]);
        // Step6: InSetProtocol
        queue_command_response(&transport, &[0xD7, 0x03, 0x00]);

        let chipset = Chipset::new(transport);
        let result = chipset.initialize();
        assert!(result.is_ok(), "initialize failed: {:?}", result);
    }

    #[test]
    // Chipset::felica_polling でカード検出 (nfcpy sense_ttf: InSetRF → InSetProtocol×2 → InCommRF)
    fn chipset_felica_polling_card_detected() {
        let transport = MockTransport::new();

        // InSetRF(212F)
        queue_command_response(&transport, &[0xD7, 0x01, 0x00]);
        // InSetProtocol(defaults)
        queue_command_response(&transport, &[0xD7, 0x03, 0x00]);
        // InSetProtocol(initial_guard_time=24)
        queue_command_response(&transport, &[0xD7, 0x03, 0x00]);

        // InCommRF → ACK + IDm = [01, FE, 01, 00, 11, 22, 33, 44]
        let payload = vec![
            0xD7, 0x05, 0x00, 0x00, 0x00, 0x00, // S1-S4 = success
            0x01, // rec_no = 1 frame received
            0x11, // SensF frame_len = 17
            0x01, // SensF resp_code
            0x01, 0xFE, 0x01, 0x00, 0x11, 0x22, 0x33, 0x44, // IDm (8 bytes)
            0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, // PMm (8 bytes)
        ];
        queue_command_response(&transport, &payload);

        let chipset = Chipset::new(transport);
        let result = chipset.felica_polling(0xFFFF, 110);
        assert!(result.is_ok());
        let idm = result.unwrap();
        assert_eq!(idm, Some("01FE010011223344".to_string()));
    }

    #[test]
    // Chipset::felica_polling でカード未検出 (S1=0x80 = タイムアウト)
    fn chipset_felica_polling_no_card() {
        let transport = MockTransport::new();

        // InSetRF(212F)
        queue_command_response(&transport, &[0xD7, 0x01, 0x00]);
        // InSetProtocol(defaults)
        queue_command_response(&transport, &[0xD7, 0x03, 0x00]);
        // InSetProtocol(initial_guard_time=24)
        queue_command_response(&transport, &[0xD7, 0x03, 0x00]);

        // InCommRF → タイムアウト: S1=0x80
        let payload = vec![0xD7, 0x05, 0x80, 0x00, 0x00, 0x00];
        queue_command_response(&transport, &payload);

        let chipset = Chipset::new(transport);
        let result = chipset.felica_polling(0xFFFF, 110);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
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

        println!("カードをタッチしてください... (10秒待機, 1:9比率ポーリング)");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut detected = false;
        let mut counter = 0u32;
        while std::time::Instant::now() < deadline {
            let (sc, timeout) = if counter % 10 == 0 {
                (0xFFFF_u16, 100_u16)
            } else {
                (0x0003_u16, 900_u16)
            };
            counter += 1;
            match chipset.felica_polling(sc, timeout) {
                Ok(Some(idm)) => {
                    println!("✓ カード検出 (sc=0x{:04X}): IDm={}", sc, idm);
                    detected = true;
                    break;
                }
                Ok(None) => {}
                Err(e) => {
                    println!("  ポーリング error: {:?}", e);
                }
            }
        }
        if !detected {
            println!("  (カードなしでシャットダウン)");
        }

        match chipset.shutdown() {
            Ok(()) => println!("✓ シャットダウン成功"),
            Err(e) => panic!("シャットダウン失敗: {:?}", e),
        }
    }
}
