/// NFC Port-100 フレームの encoding/decoding
///
/// フレーム種別:
///   ACK      : [00 00 FF 00 FF 00] (6B)
///   Error    : [00 00 FF 01 FF 7F 81 00] (8B, application-level error)
///   Normal   : [00 00 FF LEN LCS PAYLOAD... DCS 00] (≥8B, LEN < 0xFF)
///   Extended : [00 00 FF FF FF LEN_LO LEN_HI LEN_CHK PAYLOAD... DCS 00] (≥10B)
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedFrame {
    /// ACK フレーム (データなし)
    Ack,
    /// デバイスからの application-level error (payload = [0x7F])
    Error,
    /// 通常 / 拡張フレームの payload
    Data(Vec<u8>),
}

#[derive(Debug, Error)]
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

/// NFC Port-100 フレームをエンコードする。
/// payload は [D6, CMD, ...] 形式のコマンドデータ。
/// RC-S380 は拡張フレーム（FF FF マーカー付き）のみを入力として受け付ける。
pub fn encode(payload: &[u8]) -> Vec<u8> {
    let len_lo = (payload.len() & 0xFF) as u8;
    let len_hi = ((payload.len() >> 8) & 0xFF) as u8;
    let len_chk = ((0x100u16 - (len_lo as u16 + len_hi as u16)) & 0xFF) as u8;

    let data_chk_sum: u16 = payload.iter().map(|b| *b as u16).sum();
    let data_chk = ((0x100u16 - (data_chk_sum & 0xFF)) & 0xFF) as u8;

    let mut frame = Vec::with_capacity(payload.len() + 11);
    frame.extend_from_slice(&[0x00, 0x00, 0xFF, 0xFF, 0xFF]);
    frame.push(len_lo);
    frame.push(len_hi);
    frame.push(len_chk);
    frame.extend_from_slice(payload);
    frame.push(data_chk);
    frame.push(0x00);

    frame
}

/// 受信バイト列から NFC Port-100 フレームをデコードする。
pub fn decode(data: &[u8]) -> Result<DecodedFrame, FrameError> {
    // ACK フレーム判定 (6 bytes, 固定パターン)
    if data.len() >= 6 && data[..6] == [0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00] {
        return Ok(DecodedFrame::Ack);
    }

    // Error Frame 判定 (8 bytes, 固定パターン) — 通常フレームの特殊ケースだが先に確定させる
    if data.len() >= 8 && data[..8] == [0x00, 0x00, 0xFF, 0x01, 0xFF, 0x7F, 0x81, 0x00] {
        return Ok(DecodedFrame::Error);
    }

    if data.len() < 6 {
        return Err(FrameError::TooShort(data.len()));
    }

    if data[..3] != [0x00, 0x00, 0xFF] {
        return Err(FrameError::InvalidPreamble);
    }

    // 拡張フレーム vs 通常フレームの分岐
    if data.len() >= 5 && data[3] == 0xFF && data[4] == 0xFF {
        decode_extended(data)
    } else {
        decode_normal(data)
    }
}

fn decode_normal(data: &[u8]) -> Result<DecodedFrame, FrameError> {
    // data[3] = LEN, data[4] = LCS
    let len = data[3] as usize;
    let lcs = data[4];

    if ((data[3] as u16 + lcs as u16) & 0xFF) != 0 {
        return Err(FrameError::LengthChecksumMismatch);
    }

    // preamble(3) + LEN(1) + LCS(1) + payload(len) + DCS(1) + postamble(1)
    let total = 3 + 1 + 1 + len + 1 + 1;
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

fn decode_extended(data: &[u8]) -> Result<DecodedFrame, FrameError> {
    // preamble(3) + FF FF(2) + LEN_LO LEN_HI LEN_CHK(3) + payload + DCS(1) + postamble(1) = 10 min
    if data.len() < 10 {
        return Err(FrameError::TooShort(data.len()));
    }

    let len_lo = data[5];
    let len_hi = data[6];
    let len_chk = data[7];

    if ((len_lo as u16 + len_hi as u16 + len_chk as u16) & 0xFF) != 0 {
        return Err(FrameError::LengthChecksumMismatch);
    }

    let payload_len = (len_lo as usize) | ((len_hi as usize) << 8);

    let expected_total = 5 + 3 + payload_len + 1 + 1;
    if data.len() < expected_total {
        return Err(FrameError::TooShort(data.len()));
    }

    let payload = &data[8..8 + payload_len];
    let data_chk = data[8 + payload_len];

    let payload_sum: u16 = payload.iter().map(|b| *b as u16).sum();
    if ((payload_sum + data_chk as u16) & 0xFF) != 0 {
        return Err(FrameError::DataChecksumMismatch);
    }

    Ok(DecodedFrame::Data(payload.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // 空 payload を拡張フレームでエンコードする。
    fn encodes_empty_payload() {
        let payload = [];
        let frame = encode(&payload);
        // 拡張フレーム: [00 00 FF FF FF LEN_LO LEN_HI LEN_CHK DCS 00] = 10 bytes
        assert_eq!(frame.len(), 10);
        assert_eq!(frame[0..5], [0x00, 0x00, 0xFF, 0xFF, 0xFF]);
        assert_eq!(frame[5], 0x00); // LEN_LO
        assert_eq!(frame[6], 0x00); // LEN_HI
        assert_eq!(frame[7], 0x00); // LEN_CHK
        assert_eq!(frame[8], 0x00); // DCS
        assert_eq!(frame[9], 0x00); // postamble
    }

    #[test]
    // [D6, 2A, 01, 03] を拡張フレームでエンコードする。
    fn encodes_sample_payload() {
        let payload = [0xD6, 0x2A, 0x01, 0x03];
        let frame = encode(&payload);

        assert_eq!(frame[0..5], [0x00, 0x00, 0xFF, 0xFF, 0xFF]);
        assert_eq!(frame[5], 0x04); // LEN_LO = 4
        assert_eq!(frame[6], 0x00); // LEN_HI = 0
        assert_eq!(frame[7], 0xFC); // LEN_CHK = 0x100 - 0x04
        assert_eq!(&frame[8..12], &[0xD6, 0x2A, 0x01, 0x03]);
        assert_eq!(frame[12], 0xFC); // DCS
        assert_eq!(frame[13], 0x00); // postamble
    }

    #[test]
    // decode(encode(payload)) が round-trip する（通常フレーム）。
    fn roundtrip_encode_decode() {
        let payloads: Vec<Vec<u8>> = vec![
            vec![],
            vec![0xD6, 0x2A, 0x01, 0x03],
            vec![0xD6, 0x06, 0x00],
            vec![0xD6, 0x02, 0x00, 0x18, 0x01],
        ];

        for payload in payloads {
            let frame = encode(&payload);
            let decoded = decode(&frame).expect("decode failed");
            assert_eq!(
                decoded,
                DecodedFrame::Data(payload.clone()),
                "roundtrip failed for {:?}",
                payload
            );
        }
    }

    #[test]
    // decode に ACK フレーム [00, 00, FF, 00, FF, 00] → DecodedFrame::Ack
    fn decodes_ack_frame() {
        let ack = [0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00];
        let result = decode(&ack).expect("decode failed");
        assert_eq!(result, DecodedFrame::Ack);
    }

    #[test]
    // [00 00 FF 01 FF 7F 81 00] は application-level error フレームとして識別される。
    fn decodes_application_error_frame() {
        let error_frame = [0x00, 0x00, 0xFF, 0x01, 0xFF, 0x7F, 0x81, 0x00];
        let result = decode(&error_frame).expect("decode failed");
        assert_eq!(result, DecodedFrame::Error);
    }

    #[test]
    // LEN=1 の通常フレーム（payload=[0x7E]）を DecodedFrame::Data として返す。
    fn decodes_normal_frame_with_single_byte_payload() {
        // preamble: 00 00 FF
        // LEN=0x01, LCS=0xFF (01+FF=0x100)
        // payload=0x7E, DCS=0x82 (7E+82=0x100)
        // postamble: 00
        let frame = [0x00, 0x00, 0xFF, 0x01, 0xFF, 0x7E, 0x82, 0x00];
        let result = decode(&frame).expect("decode failed");
        assert_eq!(result, DecodedFrame::Data(vec![0x7E]));
    }

    #[test]
    // LEN=5 の通常フレームを payload 配列として返す。
    fn decodes_normal_frame_with_multibyte_payload() {
        // payload = [0xD7, 0x0B, 0x00, 0x01, 0x02]
        // LEN = 0x05, LCS = 0xFB (05+FB=0x100)
        // DCS = 0x100 - (D7+0B+00+01+02) = 0x100 - 0xE5 = 0x1B
        let payload = [0xD7u8, 0x0B, 0x00, 0x01, 0x02];
        let len: u8 = payload.len() as u8;
        let lcs: u8 = ((0x100u16 - len as u16) & 0xFF) as u8;
        let sum: u16 = payload.iter().map(|b| *b as u16).sum();
        let dcs: u8 = ((0x100u16 - (sum & 0xFF)) & 0xFF) as u8;

        let mut frame = vec![0x00, 0x00, 0xFF, len, lcs];
        frame.extend_from_slice(&payload);
        frame.push(dcs);
        frame.push(0x00);

        let result = decode(&frame).expect("decode failed");
        assert_eq!(result, DecodedFrame::Data(payload.to_vec()));
    }

    #[test]
    // decode に短すぎるデータ → TooShort エラー
    fn decode_too_short() {
        let short = [0x00, 0x00, 0xFF];
        let result = decode(&short);
        assert!(matches!(result, Err(FrameError::TooShort(3))));
    }

    #[test]
    // decode に壊れたプリアンブル → InvalidPreamble エラー
    fn decode_invalid_preamble() {
        let invalid = [
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let result = decode(&invalid);
        assert!(matches!(result, Err(FrameError::InvalidPreamble)));
    }

    #[test]
    // 通常フレームの LCS 不整合 → LengthChecksumMismatch エラー
    fn decode_normal_frame_length_checksum_mismatch() {
        // LEN=0x01, LCS=0x00 (1+0 != 0 mod 256)
        let frame = [0x00, 0x00, 0xFF, 0x01, 0x00, 0x7E, 0x82, 0x00];
        let result = decode(&frame);
        assert!(matches!(result, Err(FrameError::LengthChecksumMismatch)));
    }

    #[test]
    // 通常フレームの DCS 不整合 → DataChecksumMismatch エラー
    fn decode_normal_frame_data_checksum_mismatch() {
        // LEN=0x01, LCS=0xFF, payload=0x7E, DCS=0x00 (bad)
        let frame = [0x00, 0x00, 0xFF, 0x01, 0xFF, 0x7E, 0x00, 0x00];
        let result = decode(&frame);
        assert!(matches!(result, Err(FrameError::DataChecksumMismatch)));
    }

    #[test]
    // 通常フレームで LEN > 0 かつ data が短すぎる → TooShort エラー
    fn decode_normal_frame_too_short_for_payload() {
        // LEN=0x05, LCS=0xFB だが payload が 3 bytes しかない
        let frame = [0x00, 0x00, 0xFF, 0x05, 0xFB, 0xD7, 0x0B, 0x00];
        let result = decode(&frame);
        assert!(matches!(result, Err(FrameError::TooShort(_))));
    }

    #[test]
    // 通常フレームの LCS 不整合（encode 経由） → LengthChecksumMismatch エラー
    fn decode_length_checksum_mismatch() {
        // 通常フレーム: [00 00 FF 04 FC D6 2A 01 03 FC 00]
        // LCS は index 4
        let mut frame = encode(&[0xD6, 0x2A, 0x01, 0x03]);
        frame[4] = 0x00; // 正しい LCS (0xFC) を 0x00 に変更
        let result = decode(&frame);
        assert!(matches!(result, Err(FrameError::LengthChecksumMismatch)));
    }

    #[test]
    // 拡張フレームの LEN_CHK 不整合 → LengthChecksumMismatch エラー
    fn decode_extended_length_checksum_mismatch() {
        // 手動で拡張フレームを構築してテスト
        // [00 00 FF FF FF 04 00 FC D6 2A 01 03 FC 00]
        let mut frame = vec![
            0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x04, 0x00, 0xFC, // LEN_LO, LEN_HI, LEN_CHK
            0xD6, 0x2A, 0x01, 0x03, // payload
            0xFC, 0x00, // DCS, postamble
        ];
        frame[7] = 0x00; // LEN_CHK を破壊
        let result = decode(&frame);
        assert!(matches!(result, Err(FrameError::LengthChecksumMismatch)));
    }

    #[test]
    // 通常フレームの DATA_CHK 不整合 → DataChecksumMismatch エラー
    fn decode_data_checksum_mismatch() {
        let mut frame = encode(&[0xD6, 0x2A, 0x01, 0x03]);
        let data_chk_pos = frame.len() - 2; // postamble の直前
        frame[data_chk_pos] = 0x00;
        let result = decode(&frame);
        assert!(matches!(result, Err(FrameError::DataChecksumMismatch)));
    }
}
