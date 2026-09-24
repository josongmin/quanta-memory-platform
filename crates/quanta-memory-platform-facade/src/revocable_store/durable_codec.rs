//! Audited, versioned codec for the common `String` / `Vec<u8>` revocable
//! entry. It has no caller callbacks and allocates only the declared output
//! (encode) or the two charged resident values (decode). This is a logical
//! byte-budget contract, not a physical RSS bound on allocator overhead.

use quanta_memory_platform_core::{MemoryError, MemoryResult};

const MAGIC: &[u8; 4] = b"QMR1";
const HEADER_BYTES: usize = 20;

pub(super) struct DecodedLayout<'a> {
    key: &'a str,
    value: &'a [u8],
    pub charged_bytes: u64,
}

pub(super) fn encoded_len(key: &str, value: &[u8], max_record_bytes: u64) -> MemoryResult<usize> {
    let len = HEADER_BYTES
        .checked_add(key.len())
        .and_then(|len| len.checked_add(value.len()))
        .ok_or_else(|| MemoryError::capacity("durable codec record length overflow"))?;
    let bytes = u64::try_from(len)
        .map_err(|_| MemoryError::capacity("durable codec record length exceeds u64"))?;
    if bytes > max_record_bytes {
        return Err(MemoryError::capacity(format!(
            "durable codec record requires {bytes} bytes, above backend {max_record_bytes}-byte cap"
        )));
    }
    Ok(len)
}

pub(super) fn encode(key: &str, value: &[u8], len: usize) -> MemoryResult<Vec<u8>> {
    let key_len = u64::try_from(key.len())
        .map_err(|_| MemoryError::capacity("durable codec key length exceeds u64"))?;
    let value_len = u64::try_from(value.len())
        .map_err(|_| MemoryError::capacity("durable codec value length exceeds u64"))?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(len).map_err(|error| {
        MemoryError::capacity(format!("durable codec output allocation failed: {error}"))
    })?;
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&key_len.to_le_bytes());
    bytes.extend_from_slice(&value_len.to_le_bytes());
    bytes.extend_from_slice(key.as_bytes());
    bytes.extend_from_slice(value);
    debug_assert_eq!(bytes.len(), len);
    Ok(bytes)
}

pub(super) fn inspect(bytes: &[u8]) -> MemoryResult<DecodedLayout<'_>> {
    let header = bytes.get(..HEADER_BYTES).ok_or_else(|| {
        MemoryError::InvalidState("durable codec record is shorter than its header".into())
    })?;
    if &header[..4] != MAGIC {
        return Err(MemoryError::InvalidState("durable codec version/magic mismatch".into()));
    }
    let key_len = u64::from_le_bytes(header[4..12].try_into().expect("eight-byte header field"));
    let value_len = u64::from_le_bytes(header[12..20].try_into().expect("eight-byte header field"));
    let key_len = usize::try_from(key_len)
        .map_err(|_| MemoryError::capacity("durable codec key length exceeds usize"))?;
    let value_len = usize::try_from(value_len)
        .map_err(|_| MemoryError::capacity("durable codec value length exceeds usize"))?;
    let expected = HEADER_BYTES
        .checked_add(key_len)
        .and_then(|len| len.checked_add(value_len))
        .ok_or_else(|| MemoryError::capacity("durable codec decoded lengths overflow"))?;
    if expected != bytes.len() {
        return Err(MemoryError::InvalidState("durable codec record length mismatch".into()));
    }
    let key_end = HEADER_BYTES + key_len;
    let key = std::str::from_utf8(&bytes[HEADER_BYTES..key_end])
        .map_err(|_| MemoryError::InvalidState("durable codec key is not UTF-8".into()))?;
    let value = &bytes[key_end..];
    let charged_bytes = super::cost::entry_cost(
        u64::try_from(key_len)
            .map_err(|_| MemoryError::capacity("durable codec key length exceeds u64"))?,
        u64::try_from(value_len)
            .map_err(|_| MemoryError::capacity("durable codec value length exceeds u64"))?,
    )?;
    Ok(DecodedLayout { key, value, charged_bytes })
}

pub(super) fn inspect_bounded(
    bytes: &[u8],
    max_record_bytes: u64,
) -> MemoryResult<DecodedLayout<'_>> {
    let len = u64::try_from(bytes.len())
        .map_err(|_| MemoryError::capacity("durable record length exceeds u64"))?;
    if len > max_record_bytes {
        return Err(MemoryError::capacity("durable record exceeds backend cap"));
    }
    inspect(bytes)
}

pub(super) fn decode(layout: DecodedLayout<'_>) -> MemoryResult<(String, Vec<u8>)> {
    let mut key = String::new();
    key.try_reserve_exact(layout.key.len()).map_err(|error| {
        MemoryError::capacity(format!("durable codec key allocation failed: {error}"))
    })?;
    key.push_str(layout.key);
    let mut value = Vec::new();
    value.try_reserve_exact(layout.value.len()).map_err(|error| {
        MemoryError::capacity(format!("durable codec value allocation failed: {error}"))
    })?;
    value.extend_from_slice(layout.value);
    Ok((key, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_v1_record_decodes_without_using_the_current_encoder() {
        // Persisted format fixture: QMR1, little-endian key/value lengths,
        // then UTF-8 key and raw value. A future encoder change cannot update
        // this byte sequence implicitly.
        const V1: [u8; 22] =
            [b'Q', b'M', b'R', b'1', 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, b'k', b'v'];
        let layout = inspect_bounded(&V1, 22).expect("literal v1 layout");
        assert_eq!(layout.charged_bytes, 34);
        assert_eq!(decode(layout).expect("literal v1 decode"), ("k".into(), b"v".to_vec()));
        assert!(matches!(inspect_bounded(&V1, 21), Err(MemoryError::Capacity(_))));
    }

    #[test]
    fn versioned_codec_round_trip_and_exact_cost() {
        let len = encoded_len("키", b"payload", 30).expect("wire length");
        assert_eq!(len, 30);
        let bytes = encode("키", b"payload", len).expect("encode");
        let layout = inspect(&bytes).expect("inspect without allocation");
        assert_eq!(layout.charged_bytes, 42);
        assert_eq!(decode(layout).expect("decode"), ("키".into(), b"payload".to_vec()));
        assert!(matches!(encoded_len("키", b"payload", 29), Err(MemoryError::Capacity(_))));
    }

    #[test]
    fn malformed_or_forged_lengths_never_reach_allocation() {
        let valid = encode("k", b"v", 22).expect("encode");
        for bytes in [
            valid[..19].to_vec(),
            {
                let mut v = valid.clone();
                v[0] = b'X';
                v
            },
            {
                let mut v = valid.clone();
                v[4..12].copy_from_slice(&u64::MAX.to_le_bytes());
                v
            },
            {
                let mut v = valid.clone();
                v[20] = 0xff;
                v
            },
        ] {
            assert!(inspect(&bytes).is_err(), "malformed record passed preflight");
        }
    }
}
