//! When is a download finished (spec 5.1)? Its size has been still for a few
//! seconds and no other program holds it open. The retry schedule for files
//! that are not ready lives here too, as pure functions the worker calls.

use std::path::Path;
use std::time::{Duration, Instant};

pub const QUIET_PERIOD: Duration = Duration::from_secs(5);
pub const GIVE_UP_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// What the worker remembers about a file it is waiting on.
#[derive(Debug, Clone)]
pub struct Pending {
    pub first_seen: Instant,
    pub last_size: u64,
    pub last_change: Instant,
    pub attempts: u32,
    pub next_try: Instant,
}

impl Pending {
    pub fn new(size: u64, now: Instant) -> Self {
        Self { first_seen: now, last_size: size, last_change: now, attempts: 0, next_try: now + QUIET_PERIOD }
    }

    /// Records a fresh size observation; a change resets the quiet period.
    pub fn observe(&mut self, size: u64, now: Instant) {
        if size != self.last_size {
            self.last_size = size;
            self.last_change = now;
        }
    }

    pub fn is_quiet(&self, now: Instant) -> bool {
        now.duration_since(self.last_change) >= QUIET_PERIOD
    }

    /// Schedules the next look after a failed readiness check.
    pub fn defer(&mut self, now: Instant) {
        self.attempts += 1;
        self.next_try = now + next_delay(self.attempts);
    }

    pub fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.first_seen) >= GIVE_UP_AFTER
    }
}

/// 5 s, 5 s, 10 s, 20 s, 60 s, 2 min, then every 15 min. The first retries are
/// quick because an antivirus scan of a fresh download holds it only briefly;
/// the long tail covers a file she opened straight away in Word.
pub fn next_delay(attempt: u32) -> Duration {
    match attempt {
        0..=2 => Duration::from_secs(5),
        3 => Duration::from_secs(10),
        4 => Duration::from_secs(20),
        5 => Duration::from_secs(60),
        6 => Duration::from_secs(120),
        _ => Duration::from_secs(15 * 60),
    }
}

/// True when the file can be opened with no sharing, meaning no other program
/// has it open. On other platforms a plain open stands in.
pub fn can_open_exclusively(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).share_mode(0).open(path).is_ok()
    }
    #[cfg(not(windows))]
    {
        std::fs::File::open(path).is_ok()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn quiet_after_the_period_without_size_changes() {
        let start = Instant::now();
        let mut pending = Pending::new(100, start);
        assert!(!pending.is_quiet(start + Duration::from_secs(2)));
        pending.observe(200, start + Duration::from_secs(3));
        assert!(!pending.is_quiet(start + Duration::from_secs(7)));
        assert!(pending.is_quiet(start + Duration::from_secs(9)));
    }

    #[test]
    fn backoff_grows_then_plateaus() {
        assert_eq!(next_delay(1), Duration::from_secs(5));
        assert_eq!(next_delay(2), Duration::from_secs(5));
        assert_eq!(next_delay(3), Duration::from_secs(10));
        assert_eq!(next_delay(5), Duration::from_secs(60));
        assert_eq!(next_delay(7), Duration::from_secs(900));
        assert_eq!(next_delay(40), Duration::from_secs(900));
        let start = Instant::now();
        let mut pending = Pending::new(1, start);
        pending.defer(start);
        assert_eq!(pending.attempts, 1);
        assert!(!pending.expired(start + Duration::from_secs(3600)));
        assert!(pending.expired(start + GIVE_UP_AFTER));
    }

    #[test]
    fn a_closed_file_opens_exclusively() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("done.docx");
        std::fs::write(&file, b"x").unwrap();
        assert!(can_open_exclusively(&file));
    }
}
