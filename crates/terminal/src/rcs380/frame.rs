/// NFC Port-100 フレームの encoding/decoding
///
/// フレーム構造:
/// [00] [00] [FF] [FF] [FF] [LEN_LO] [LEN_HI] [LEN_CHK] [PAYLOAD...] [DATA_CHK] [00]

use thiserror::Error;

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
/// ACK フレーム [00, 00, FF, 00, FF, 00] の場合は Ok(None) を返す。
/// 正常フレームの場合は payload 部分を返す。
pub fn decode(data: &[u8]) -> Result<Option<Vec<u8>>, FrameError> {
    // ACK フレーム判定 (6 bytes)
    if data.len() == 6 && data == &[0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00] {
        return Ok(None);
    }

    // 最小フレーム長は 10 bytes (3 + 2 + 3 + 0 + 1 + 1)
    if data.len() < 10 {
        return Err(FrameError::TooShort(data.len()));
    }

    // プリアンブル検証
    if &data[0..3] != &[0x00, 0x00, 0xFF] {
        return Err(FrameError::InvalidPreamble);
    }

    // 拡張マーカ検証
    if &data[3..5] != &[0xFF, 0xFF] {
        return Err(FrameError::InvalidPreamble);
    }

    let len_lo = data[5];
    let len_hi = data[6];
    let len_chk = data[7];

    // LEN_CHK 検証: (len_lo + len_hi + len_chk) & 0xFF == 0
    if ((len_lo as u16 + len_hi as u16 + len_chk as u16) & 0xFF) != 0 {
        return Err(FrameError::LengthChecksumMismatch);
    }

    let payload_len = (len_lo as usize) | ((len_hi as usize) << 8);

    // フレーム全体の長さを検証
    let expected_total = 5 + 3 + payload_len + 1 + 1; // preamble + len_fields + payload + data_chk + postamble
    if data.len() < expected_total {
        return Err(FrameError::TooShort(data.len()));
    }

    let payload = &data[8..8 + payload_len];
    let data_chk = data[8 + payload_len];

    // DATA_CHK 検証: (payload の合計 + data_chk) & 0xFF == 0
    let payload_sum: u16 = payload.iter().map(|b| *b as u16).sum();
    if ((payload_sum + data_chk as u16) & 0xFF) != 0 {
        return Err(FrameError::DataChecksumMismatch);
    }

    Ok(Some(payload.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // 空 payload をエンコードする。
    fn encodes_empty_payload() {
        let payload = [];
        let frame = encode(&payload);
        // 最小フレーム: [00 00 FF FF FF 00 00 00 00 00] (長さ10)
        assert_eq!(frame.len(), 10);
        assert_eq!(frame[0..5], [0x00, 0x00, 0xFF, 0xFF, 0xFF]);
        assert_eq!(frame[5], 0x00); // len_lo
        assert_eq!(frame[6], 0x00); // len_hi
        assert_eq!(frame[7], 0x00); // len_chk
        assert_eq!(frame[8], 0x00); // data_chk (0 の 2 の補数)
        assert_eq!(frame[9], 0x00); // postamble
    }

    #[test]
    // [D6, 2A, 01, 03] をエンコードする。
    fn encodes_sample_payload() {
        let payload = [0xD6, 0x2A, 0x01, 0x03];
        let frame = encode(&payload);

        assert_eq!(frame[0..5], [0x00, 0x00, 0xFF, 0xFF, 0xFF]);
        assert_eq!(frame[5], 0x04); // len_lo = 4
        assert_eq!(frame[6], 0x00); // len_hi = 0
        assert_eq!(frame[7], 0xFC); // len_chk = 0x100 - 0x04 = 0xFC
        assert_eq!(&frame[8..12], &[0xD6, 0x2A, 0x01, 0x03]); // payload
        // data_chk: 0x100 - (0xD6 + 0x2A + 0x01 + 0x03) = 0x100 - 0x104 = 0xFC
        assert_eq!(frame[12], 0xFC);
        assert_eq!(frame[13], 0x00); // postamble
    }

    #[test]
    // decode(encode(payload)) が round-trip する。
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
            assert_eq!(decoded, Some(payload.clone()), "roundtrip failed for {:?}", payload);
        }
    }

    #[test]
    // decode に ACK フレーム [00, 00, FF, 00, FF, 00] → Ok(None)
    fn decodes_ack_frame() {
        let ack = [0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00];
        let result = decode(&ack).expect("decode failed");
        assert_eq!(result, None);
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
        let invalid = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let result = decode(&invalid);
        assert!(matches!(result, Err(FrameError::InvalidPreamble)));
    }

    #[test]
    // decode に壊れた LEN_CHK → LengthChecksumMismatch エラー
    fn decode_length_checksum_mismatch() {
        let mut frame = encode(&[0xD6, 0x2A, 0x01, 0x03]);
        frame[7] = 0x00; // 正しい len_chk (0xFC) を 0x00 に変更
        let result = decode(&frame);
        assert!(matches!(result, Err(FrameError::LengthChecksumMismatch)));
    }

    #[test]
    // decode に壊れた DATA_CHK → DataChecksumMismatch エラー
    fn decode_data_checksum_mismatch() {
        let mut frame = encode(&[0xD6, 0x2A, 0x01, 0x03]);
        // フレーム構造: [00 00 FF FF FF LEN_LO LEN_HI LEN_CHK PAYLOAD DATA_CHK 00]
        // payload length = 4, so DATA_CHK is at index 8 + 4 = 12
        let data_chk_pos = frame.len() - 2; // postamble の直前
        frame[data_chk_pos] = 0x00; // 正しい data_chk を 0x00 に変更
        let result = decode(&frame);
        assert!(matches!(result, Err(FrameError::DataChecksumMismatch)));
    }
}
