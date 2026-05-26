/// Resume token encoding and decoding for Firestore Listen streams.
///
/// Token layout: [8-byte i64 BE unix seconds][32-byte BLAKE3(ts_bytes || project_id bytes)] = 40 bytes total
///
/// On reconnect with a token <= 24h old: query only docs with update_time > decoded_ts (delta only).
/// Token > 24h or malformed: full re-snapshot (no error).
use blake3::Hasher;
use chrono::{DateTime, Duration, Utc};

/// Encode a resume token from a timestamp and project_id.
pub fn encode(ts: DateTime<Utc>, project_id: &str) -> Vec<u8> {
    let ts_secs = ts.timestamp();
    let ts_bytes = ts_secs.to_be_bytes();
    let mut hasher = Hasher::new();
    hasher.update(&ts_bytes);
    hasher.update(project_id.as_bytes());
    let hash = hasher.finalize();
    let mut out = Vec::with_capacity(40);
    out.extend_from_slice(&ts_bytes);
    out.extend_from_slice(hash.as_bytes());
    out
}

/// Decode the timestamp from a resume token.
/// Returns None if the token is too short or otherwise malformed.
pub fn decode_ts(token: &[u8]) -> Option<DateTime<Utc>> {
    if token.len() < 8 {
        return None;
    }
    let ts_bytes: [u8; 8] = token[..8].try_into().ok()?;
    let ts = i64::from_be_bytes(ts_bytes);
    DateTime::from_timestamp(ts, 0)
}

/// Returns true if the resume token is older than 24 hours or malformed.
pub fn is_stale(token: &[u8]) -> bool {
    match decode_ts(token) {
        Some(ts) => Utc::now() - ts > Duration::hours(24),
        None => true,
    }
}
