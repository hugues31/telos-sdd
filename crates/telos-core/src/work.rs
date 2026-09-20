//! Shared identities, timestamps and canonical hashes for durable work records.

use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::error::{ErrorCode, TelosError};

pub fn new_id(prefix: &str) -> Result<String, TelosError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| {
        TelosError::new(
            ErrorCode::TelosInternal,
            format!("cannot allocate an identity: {e}"),
        )
    })?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let h = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok(format!(
        "{prefix}-{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}

pub fn validate_id(prefix: &str, id: &str) -> Result<(), TelosError> {
    let valid = id.strip_prefix(&format!("{prefix}-")).is_some_and(|s| {
        s.len() == 36
            && s.bytes().enumerate().all(|(i, c)| {
                if [8, 13, 18, 23].contains(&i) {
                    c == b'-'
                } else {
                    c.is_ascii_digit() || (b'a'..=b'f').contains(&c)
                }
            })
    });
    if valid {
        Ok(())
    } else {
        Err(TelosError::new(
            ErrorCode::TelosParseError,
            format!("expected {prefix}-<lowercase UUID>, got `{id}`"),
        ))
    }
}

pub fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn digest(value: &impl Serialize) -> Result<String, TelosError> {
    // Going through Value sorts object keys regardless of struct field order.
    let canonical = serde_json::to_value(value)
        .and_then(|v| serde_json::to_vec(&v))
        .map_err(|e| {
            TelosError::new(
                ErrorCode::TelosInternal,
                format!("cannot serialize work record: {e}"),
            )
        })?;
    Ok(hash(&canonical))
}

pub fn now() -> String {
    timestamp(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
}

fn timestamp(seconds: u64) -> String {
    let days = (seconds / 86400) as i64 + 719468;
    let era = days / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_timestamps_cover_epoch_and_leap_day() {
        assert_eq!(timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(timestamp(1709164800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn identities_are_unique_and_cannot_escape_paths() {
        let first = new_id("PLN").unwrap();
        validate_id("PLN", &first).unwrap();
        assert_ne!(first, new_id("PLN").unwrap());
        for value in [
            "PLN-../escape",
            "PLN-0001",
            "PLN-00000000-0000-0000-0000-00000000000/",
        ] {
            assert!(validate_id("PLN", value).is_err());
        }
    }
}
