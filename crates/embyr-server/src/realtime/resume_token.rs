/// Resume token encoding and decoding for Firestore Listen streams.
///
/// Token layout: [8-byte i64 BE unix seconds] = 8 bytes total.
///
/// The token is opaque, unsigned data returned to and replayed by a client
/// already authenticated to a single project's own Listen stream (the
/// project scope for a resumed query always comes from the request's own
/// target, never from the token). A forged/tampered timestamp therefore
/// cannot cross project boundaries; the worst case is a caller forcing a
/// larger-than-necessary delta scan against their own already-authorized
/// project, bounded by the 24h staleness window below. Audit finding #31
/// (production-readiness-audit-2026-09-08.md): a previous revision appended
/// an unkeyed 32-byte BLAKE3 suffix that `decode_ts` never read or verified
/// — decorative weight with no security effect. Removed rather than made
/// real, since keyed verification would not reduce the (already-bounded,
/// self-inflicted-only) worst case.
///
/// On reconnect with a token <= 24h old: query only docs with update_time > decoded_ts (delta only).
/// Token > 24h or malformed: full re-snapshot (no error).
use chrono::{DateTime, Duration, Utc};

/// Encode a resume token from a timestamp.
pub fn encode(ts: DateTime<Utc>) -> Vec<u8> {
    ts.timestamp().to_be_bytes().to_vec()
}

/// Decode the timestamp from a resume token.
/// Returns None if the token is too short or otherwise malformed.
/// Tolerates trailing bytes beyond the first 8 (backward-compatible with
/// tokens issued by the pre-#31-fix 40-byte format).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_ts_round_trips_through_encode() {
        let ts = DateTime::from_timestamp(1_726_700_000, 0).unwrap();
        let token = encode(ts);
        assert_eq!(token.len(), 8, "shrunk format is 8 bytes, no BLAKE3 suffix");
        assert_eq!(decode_ts(&token), Some(ts));
    }

    #[test]
    fn decode_ts_rejects_short_token() {
        assert_eq!(decode_ts(&[1, 2, 3]), None);
        assert_eq!(decode_ts(&[]), None);
    }

    #[test]
    fn decode_ts_ignores_trailing_bytes_for_backward_compat_with_old_40_byte_tokens() {
        let ts = DateTime::from_timestamp(1_726_700_000, 0).unwrap();
        let mut legacy_token = encode(ts);
        legacy_token.extend_from_slice(&[0xAA; 32]); // old unverified BLAKE3 suffix
        assert_eq!(decode_ts(&legacy_token), Some(ts));
    }
}
