//! hourly-metrics-rollup (ADR-084): pure bucket/percentile functions and the
//! in-memory write buffer. Zero IO — the server crate flushes the buffer.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

use chrono::{DateTime, Utc};

/// Latency histogram size: 12 finite buckets plus overflow.
pub const BUCKET_COUNT: usize = 13;
/// Inclusive upper bounds in ms of buckets b00..b11; b12 is +inf.
pub const BUCKET_UPPER_MS: [f64; BUCKET_COUNT - 1] =
    [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0];

/// Operation class of a rollup row (the `op` column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    Read = 0,
    Write = 1,
    Delete = 2,
    RunQuery = 3,
}

/// Index of the histogram bucket a latency falls in.
pub fn bucket_index(latency_ms: f64) -> usize {
    BUCKET_UPPER_MS
        .iter()
        .position(|upper| latency_ms <= *upper)
        .unwrap_or(BUCKET_COUNT - 1)
}

/// UTC hour containing `ts`.
pub fn truncate_to_hour(ts: DateTime<Utc>) -> DateTime<Utc> {
    let secs = ts.timestamp();
    DateTime::from_timestamp(secs - secs.rem_euclid(3600), 0).expect("in-range timestamp")
}

/// Bucket holding the q-quantile (cumulative count >= q * total), the rank
/// consumed by the buckets before it, and the rank itself.
fn locate(buckets: &[u64; BUCKET_COUNT], q: f64) -> Option<(usize, f64, f64)> {
    let total: f64 = buckets.iter().map(|c| *c as f64).sum();
    if total == 0.0 {
        return None;
    }
    let rank = q.clamp(0.0, 1.0) * total;
    let mut before = 0.0;
    for (i, count) in buckets.iter().enumerate() {
        let count = *count as f64;
        if count == 0.0 {
            continue;
        }
        if before + count >= rank {
            return Some((i, before, rank));
        }
        before += count;
    }
    None
}

/// q-quantile (0..=1) in ms by linear interpolation inside the containing
/// bucket; `None` when there are no samples. The +inf bucket reports its
/// lower bound (5000 ms) — see [`is_saturated`].
pub fn percentile(buckets: &[u64; BUCKET_COUNT], q: f64) -> Option<f64> {
    let (i, before, rank) = locate(buckets, q)?;
    let last = BUCKET_COUNT - 1;
    if i == last {
        return Some(BUCKET_UPPER_MS[last - 1]);
    }
    let lower = if i == 0 { 0.0 } else { BUCKET_UPPER_MS[i - 1] };
    let frac = (rank - before) / buckets[i] as f64;
    Some(lower + frac * (BUCKET_UPPER_MS[i] - lower))
}

/// True when the q-quantile lies in the +inf bucket (reported value is a floor).
pub fn is_saturated(buckets: &[u64; BUCKET_COUNT], q: f64) -> bool {
    matches!(locate(buckets, q), Some((i, _, _)) if i == BUCKET_COUNT - 1)
}

/// Counters and histogram for one (project, hour, op). `requests == Σ buckets`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Acc {
    pub requests: u64,
    pub units: u64,
    pub errors: u64,
    pub buckets: [u64; BUCKET_COUNT],
}

impl Acc {
    pub fn observe(&mut self, latency_ms: f64, units: u64, is_error: bool) {
        self.requests += 1;
        self.units += units;
        self.errors += u64::from(is_error);
        self.buckets[bucket_index(latency_ms)] += 1;
    }

    pub fn merge(&mut self, other: &Acc) {
        self.requests += other.requests;
        self.units += other.units;
        self.errors += other.errors;
        for (a, b) in self.buckets.iter_mut().zip(other.buckets.iter()) {
            *a += b;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RollupKey {
    pub project_id: String,
    pub hour: DateTime<Utc>,
    pub op: Op,
}

/// Default bound on distinct (project, hour, op) keys held in memory.
pub const DEFAULT_MAX_KEYS: usize = 50_000;

/// Process-wide write buffer: updated on request completion (never awaited,
/// never touches the DB), drained by the flush task. Bounded: a new key past
/// `max_keys` is dropped and counted.
#[derive(Debug)]
pub struct RollupBuffer {
    map: Mutex<HashMap<RollupKey, Acc>>,
    max_keys: AtomicUsize,
    dropped: AtomicU64,
}

impl RollupBuffer {
    pub fn new(max_keys: usize) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            max_keys: AtomicUsize::new(max_keys),
            dropped: AtomicU64::new(0),
        }
    }

    pub fn set_max_keys(&self, max_keys: usize) {
        self.max_keys.store(max_keys, Ordering::Relaxed);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<RollupKey, Acc>> {
        self.map.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Record one completed request. Returns `false` when it was dropped
    /// because the key bound is reached.
    pub fn record(
        &self,
        project_id: &str,
        hour: DateTime<Utc>,
        op: Op,
        latency_ms: f64,
        units: u64,
        is_error: bool,
    ) -> bool {
        let key = RollupKey { project_id: project_id.to_string(), hour, op };
        let mut map = self.lock();
        if !map.contains_key(&key) && map.len() >= self.max_keys.load(Ordering::Relaxed) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return false;
        }
        map.entry(key).or_default().observe(latency_ms, units, is_error);
        true
    }

    /// Take everything buffered so far (the lock is held only for the swap).
    pub fn drain(&self) -> Vec<(RollupKey, Acc)> {
        std::mem::take(&mut *self.lock()).into_iter().collect()
    }

    /// Merge drained entries back after a failed flush. Entries for new keys
    /// past the bound are dropped; returns the number of requests dropped.
    pub fn restore(&self, entries: Vec<(RollupKey, Acc)>) -> u64 {
        let mut map = self.lock();
        let max = self.max_keys.load(Ordering::Relaxed);
        let mut dropped = 0;
        for (key, acc) in entries {
            if !map.contains_key(&key) && map.len() >= max {
                dropped += acc.requests;
            } else {
                map.entry(key).or_default().merge(&acc);
            }
        }
        self.dropped.fetch_add(dropped, Ordering::Relaxed);
        dropped
    }

    /// Requests dropped so far because of the key bound.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use proptest::prelude::*;

    fn buckets_of(samples: &[f64]) -> [u64; BUCKET_COUNT] {
        let mut b = [0u64; BUCKET_COUNT];
        for s in samples {
            b[bucket_index(*s)] += 1;
        }
        b
    }

    fn hour(h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 13, h, 0, 0).unwrap()
    }

    #[test]
    fn bucket_bounds_are_inclusive() {
        assert_eq!(bucket_index(0.0), 0);
        assert_eq!(bucket_index(1.0), 0);
        assert_eq!(bucket_index(1.01), 1);
        assert_eq!(bucket_index(5000.0), 11);
        assert_eq!(bucket_index(5000.1), 12);
        assert_eq!(bucket_index(f64::MAX), 12);
    }

    #[test]
    fn truncates_to_utc_hour() {
        let ts = Utc.with_ymd_and_hms(2026, 11, 13, 9, 59, 59).unwrap();
        assert_eq!(truncate_to_hour(ts), hour(9));
        assert_eq!(truncate_to_hour(hour(9)), hour(9));
    }

    /// AC-HMR-02: latencies {3,3,8,40,900}. The exact median is 8 ms (bucket
    /// 5-10), so p50 is asserted against that bucket, not the 2-5 ms range
    /// quoted in the AC text (no standard quantile definition yields <= 5).
    #[test]
    fn percentiles_of_known_latencies_fall_in_their_buckets() {
        let b = buckets_of(&[3.0, 3.0, 8.0, 40.0, 900.0]);
        let p50 = percentile(&b, 0.5).unwrap();
        let p95 = percentile(&b, 0.95).unwrap();
        assert!((5.0..=10.0).contains(&p50), "p50 = {p50}");
        assert!((500.0..=1000.0).contains(&p95), "p95 = {p95}");
        assert!(!is_saturated(&b, 0.99));
    }

    /// AC-HMR-03
    #[test]
    fn no_samples_means_no_percentile() {
        assert_eq!(percentile(&[0; BUCKET_COUNT], 0.95), None);
        assert!(!is_saturated(&[0; BUCKET_COUNT], 0.95));
    }

    #[test]
    fn overflow_bucket_reports_its_lower_bound_and_saturates() {
        let b = buckets_of(&[10_000.0, 20_000.0]);
        assert_eq!(percentile(&b, 0.99), Some(5000.0));
        assert!(is_saturated(&b, 0.99));
    }

    #[test]
    fn buffer_merges_counts_per_key() {
        let buf = RollupBuffer::new(10);
        buf.record("p", hour(9), Op::Read, 3.0, 2, false);
        buf.record("p", hour(9), Op::Read, 900.0, 1, true);
        buf.record("p", hour(9), Op::Write, 3.0, 1, false);
        let mut drained = buf.drain();
        drained.sort_by_key(|(k, _)| k.op as u8);
        assert_eq!(drained.len(), 2);
        let read = &drained[0].1;
        assert_eq!((read.requests, read.units, read.errors), (2, 3, 1));
        assert_eq!(read.buckets.iter().sum::<u64>(), read.requests);
        assert!(buf.is_empty());
    }

    #[test]
    fn buffer_drops_new_keys_past_bound_but_keeps_counting_existing() {
        let buf = RollupBuffer::new(1);
        assert!(buf.record("a", hour(9), Op::Read, 1.0, 1, false));
        assert!(!buf.record("b", hour(9), Op::Read, 1.0, 1, false));
        assert!(buf.record("a", hour(9), Op::Read, 1.0, 1, false));
        assert_eq!(buf.dropped(), 1);
        assert_eq!(buf.len(), 1);
        assert_eq!(buf.drain()[0].1.requests, 2);
    }

    #[test]
    fn restore_after_failed_flush_adds_back_and_stays_bounded() {
        let buf = RollupBuffer::new(1);
        buf.record("a", hour(9), Op::Read, 1.0, 1, false);
        let drained = buf.drain();
        buf.record("a", hour(9), Op::Read, 1.0, 1, false); // arrived during the failed flush
        buf.record("b", hour(9), Op::Read, 1.0, 1, false); // dropped: bound is 1
        assert_eq!(buf.restore(drained), 0);
        assert_eq!(buf.drain()[0].1.requests, 2);

        let buf = RollupBuffer::new(1);
        buf.record("x", hour(9), Op::Read, 1.0, 1, false);
        let mut entries = buf.drain();
        buf.record("y", hour(9), Op::Read, 1.0, 1, false);
        entries.push((
            RollupKey { project_id: "z".into(), hour: hour(9), op: Op::Read },
            Acc { requests: 7, ..Acc::default() },
        ));
        assert_eq!(buf.restore(entries), 8); // x (1) and z (7) are new keys, bound is full
    }

    proptest! {
        #[test]
        fn percentile_is_monotone_and_within_the_exact_quantiles_bucket(
            samples in prop::collection::vec(0.0f64..8000.0, 1..200),
            q1 in 0.0f64..=1.0,
            q2 in 0.0f64..=1.0,
        ) {
            let b = buckets_of(&samples);
            let (lo, hi) = if q1 <= q2 { (q1, q2) } else { (q2, q1) };
            prop_assert!(percentile(&b, lo).unwrap() <= percentile(&b, hi).unwrap() + 1e-9);

            let mut sorted = samples.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let k = ((q1 * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len());
            let idx = bucket_index(sorted[k - 1]);
            let lower = if idx == 0 { 0.0 } else { BUCKET_UPPER_MS[idx - 1] };
            let upper = if idx == BUCKET_COUNT - 1 { 5000.0 } else { BUCKET_UPPER_MS[idx] };
            let p = percentile(&b, q1).unwrap();
            prop_assert!(p >= lower - 1e-9 && p <= upper + 1e-9, "p={p} not in [{lower},{upper}]");
        }

        #[test]
        fn acc_requests_equal_bucket_total_after_any_merge(
            a in prop::collection::vec((0.0f64..9000.0, 0u64..50, any::<bool>()), 0..40),
            b in prop::collection::vec((0.0f64..9000.0, 0u64..50, any::<bool>()), 0..40),
        ) {
            let (mut x, mut y) = (Acc::default(), Acc::default());
            for (l, u, e) in &a { x.observe(*l, *u, *e); }
            for (l, u, e) in &b { y.observe(*l, *u, *e); }
            x.merge(&y);
            prop_assert_eq!(x.requests as usize, a.len() + b.len());
            prop_assert_eq!(x.buckets.iter().sum::<u64>(), x.requests);
        }
    }
}
