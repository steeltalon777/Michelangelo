//! Small time/id helpers shared by the PoC (no external time crate).

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// ISO-8601 UTC timestamp with second precision (same style as existing crates).
pub fn now_iso8601() -> String {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = duration.as_secs();

    let days = total_secs / 86400;
    let time_secs = total_secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }

    let month_days = if is_leap(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 1usize;
    let mut remaining_local = remaining;
    for &md in &month_days {
        if remaining_local < md {
            break;
        }
        remaining_local -= md;
        m += 1;
    }
    let d = remaining_local + 1;

    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

static TOKEN_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Process-unique random-ish token used as an owner fencing token.
///
/// Combines pid, wall-clock nanos, a process-local counter and a hash seed
/// that is randomized per process. Not cryptographic; enough to distinguish
/// owners within this PoC (documented limitation).
pub fn new_owner_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let counter = TOKEN_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(nanos.rotate_left(17));
    hasher.write_u64(counter);
    hasher.write_u64(std::process::id() as u64);
    let mixed = hasher.finish() ^ (nanos << 1) ^ counter.rotate_left(31);
    format!("{:016x}{:016x}", mixed, (!mixed).rotate_left(13))
}
