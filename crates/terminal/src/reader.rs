use jiff::{Timestamp, Zoned, tz::TimeZone};
use pasori_core::port::reader::{CardId, CardScanned, ReaderBackend, ReaderError, ReaderStatus};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// 連続スキャン抑制ウィンドウ (5 秒)
const SUPPRESSION_WINDOW_SECS: u64 = 5;
/// PC/SC ポーリング間隔
const POLL_INTERVAL_MS: u64 = 200;
/// `GET DATA` (IDm 取得) APDU
const GET_UID_APDU: [u8; 5] = [0xFF, 0xCA, 0x00, 0x00, 0x00];

pub struct PcscReaderBackend {
    status: Arc<Mutex<ReaderStatus>>,
    tx: broadcast::Sender<CardScanned>,
    handle: Mutex<Option<JoinHandle<()>>>,
    cancel: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
}

impl PcscReaderBackend {
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

impl Default for PcscReaderBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ReaderBackend for PcscReaderBackend {
    async fn start(&self) -> Result<(), ReaderError> {
        let mut handle_guard = self.handle.lock().expect("handle lock");
        if handle_guard.is_some() {
            // already running
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
        // MutexGuard を await 前にドロップするため take() だけここで行う
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

/// PC/SC ポーリングループ。`spawn_blocking` 内で動く。
fn poll_loop(
    tx: broadcast::Sender<CardScanned>,
    status: Arc<Mutex<ReaderStatus>>,
    cancel: tokio::sync::watch::Receiver<bool>,
) {
    let ctx = match pcsc::Context::establish(pcsc::Scope::User) {
        Ok(ctx) => ctx,
        Err(e) => {
            *status.lock().expect("status lock") =
                ReaderStatus::Error(format!("PC/SC context error: {e}"));
            return;
        }
    };

    // 連続スキャン抑制テーブル: card_id -> 最後にスキャンした Instant
    let mut last_seen: HashMap<String, std::time::Instant> = HashMap::new();

    loop {
        // キャンセルチェック
        if *cancel.borrow() {
            break;
        }

        // リーダー一覧取得
        let readers = match list_readers(&ctx) {
            Ok(r) => r,
            Err(e) => {
                *status.lock().expect("status lock") = ReaderStatus::Error(e.clone());
                tracing::warn!("PC/SC list readers failed: {e}");
                std::thread::sleep(Duration::from_millis(1000));
                continue;
            }
        };

        if readers.is_empty() {
            *status.lock().expect("status lock") = ReaderStatus::Disconnected;
            tracing::debug!("no PC/SC readers found, waiting...");
            std::thread::sleep(Duration::from_millis(1000));
            continue;
        }

        tracing::debug!(readers = ?readers, "PC/SC readers found");
        *status.lock().expect("status lock") = ReaderStatus::Ready;

        for reader_name in &readers {
            if *cancel.borrow() {
                return;
            }

            match read_card_id(&ctx, reader_name) {
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
                    tracing::debug!(reader = %reader_name, error = %e, "card read error (card may not be present)");
                }
            }
        }

        std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }
}

/// PC/SC コンテキストからリーダー名一覧を取得する。
fn list_readers(ctx: &pcsc::Context) -> Result<Vec<String>, String> {
    let mut buf = vec![0u8; 4096];
    let names = ctx.list_readers(&mut buf).map_err(|e| e.to_string())?;
    Ok(names
        .filter_map(|n| n.to_str().ok())
        .map(|s| s.to_string())
        .collect())
}

/// リーダーに挿入されているカードの IDm (8 バイト hex) を取得する。
/// カードが存在しなければ `Ok(None)` を返す。
fn read_card_id(ctx: &pcsc::Context, reader_name: &str) -> Result<Option<String>, ReaderError> {
    use std::ffi::CString;

    let name = CString::new(reader_name).map_err(|e| ReaderError::Other(e.to_string()))?;

    let card = match ctx.connect(&name, pcsc::ShareMode::Shared, pcsc::Protocols::ANY) {
        Ok(c) => c,
        Err(pcsc::Error::NoSmartcard) | Err(pcsc::Error::RemovedCard) => return Ok(None),
        Err(e) => return Err(ReaderError::Pcsc(e.to_string())),
    };

    let mut resp_buf = [0u8; 256];
    let resp = card
        .transmit(&GET_UID_APDU, &mut resp_buf)
        .map_err(|e| ReaderError::Pcsc(e.to_string()))?;

    // 正常応答: IDm (8 バイト) + SW1SW2 (2 バイト) = 10 バイト
    // SW=9000 が成功
    if resp.len() < 2 {
        return Err(ReaderError::Pcsc("too short response".to_string()));
    }

    let sw1 = resp[resp.len() - 2];
    let sw2 = resp[resp.len() - 1];

    if sw1 != 0x90 || sw2 != 0x00 {
        return Err(ReaderError::Pcsc(format!("SW={sw1:02X}{sw2:02X}")));
    }

    let uid_bytes = &resp[..resp.len() - 2];
    let hex = uid_bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join("");

    Ok(Some(hex))
}

fn tokyo_now() -> Zoned {
    Timestamp::now().to_zoned(TimeZone::get("Asia/Tokyo").expect("Asia/Tokyo tz"))
}

/// 接続されている NFC リーダーを自動検出し、適切なバックエンドを返す。
///
/// 検出順序:
/// 1. USB デバイス一覧から RC-S380 (VID=054C, PID=06C3) を検索 → RCS380ReaderBackend
/// 2. PC/SC リーダー一覧を検索 → PcscReaderBackend
/// 3. いずれも見つからない場合 → ReaderError::NotConnected
pub fn detect_and_create() -> Result<Box<dyn ReaderBackend>, ReaderError> {
    use crate::rcs380::chipset::RCS380ReaderBackend;

    // 1. rusb でデバイス検索
    if let Ok(devices) = rusb::devices() {
        for device in devices.iter() {
            if let Ok(desc) = device.device_descriptor() {
                if desc.vendor_id() == 0x054C && desc.product_id() == 0x06C3 {
                    tracing::info!("RC-S380 detected, using rusb backend");
                    return Ok(Box::new(RCS380ReaderBackend::new()));
                }
            }
        }
    }

    // 2. PC/SC フォールバック
    if let Ok(ctx) = pcsc::Context::establish(pcsc::Scope::User) {
        let mut buf = vec![0u8; 4096];
        if let Ok(mut readers) = ctx.list_readers(&mut buf) {
            if readers.next().is_some() {
                tracing::info!("PC/SC reader detected, using pcsc backend");
                return Ok(Box::new(PcscReaderBackend::new()));
            }
        }
    }

    Err(ReaderError::NotConnected)
}

#[cfg(test)]
mod tests {
    use super::PcscReaderBackend;
    use pasori_core::port::reader::{ReaderBackend, ReaderStatus};

    #[test]
    // 初期状態は Disconnected である。
    fn initial_status_is_disconnected() {
        let reader = PcscReaderBackend::new();
        assert_eq!(reader.status(), ReaderStatus::Disconnected);
    }

    #[test]
    // subscribe は broadcast の Receiver を返す。
    fn subscribe_returns_receiver() {
        let reader = PcscReaderBackend::new();
        let _rx = reader.subscribe();
    }
}
