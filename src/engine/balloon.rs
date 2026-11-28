//! The memory balloon: keeping the VM from growing to its full memory.
//!
//! macOS gives the VM memory as the guest first touches it, and the
//! Virtualization framework never takes it back, not even from pages the
//! guest puts in the balloon. Linux, left alone, fills all its free memory
//! with file cache, so the VM grows to its full size within a few builds or
//! pulls. What the balloon can do is keep the guest from touching memory it
//! doesn't need: memory in the balloon is out of the guest's reach. Every
//! couple of seconds the engine reads the guest's memory use over vsock
//! (`patina-memory`) and sizes the balloon so the guest keeps what it uses
//! plus room to grow:
//!
//! - when the guest needs more, it gets it at once, with more room on top;
//! - when it has needed less for [`SETTLE`], the balloon takes the rest back.
//!
//! So the VM grows to the most the guest has really used, plus room, instead
//! of its full size, until the engine restarts. Between readings, a guest
//! that would run out takes pages back from the balloon itself
//! (`VIRTIO_BALLOON_F_DEFLATE_ON_OOM`), so a sudden spike gets memory from the
//! balloon instead of an out-of-memory kill.
//!
//! The balloon only works when the guest doesn't zero pages as it allocates
//! them ([`super::guest::KERNEL_ARGS`]): zeroing the pages it takes would
//! touch them, and grow the VM by the balloon's size. A guest that zeroes
//! them, or can't take memory back from the balloon, keeps all its memory.

use std::time::{Duration, Instant};

/// How often the engine reads the guest's memory use.
pub const INTERVAL: Duration = Duration::from_secs(2);
/// How long the guest must need less memory before the balloon takes it.
pub const SETTLE: Duration = Duration::from_secs(30);
/// The least room to grow the guest keeps past what it uses.
const ROOM: u64 = 1 << 30;
/// Sizes are rounded up to this, so small changes in use don't move the balloon.
const STEP: u64 = 128 << 20;

/// The guest's memory, as `patina-memory` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stats {
    /// `MemTotal`: the memory Linux manages. With deflate-on-OOM, the
    /// balloon's pages stay in it.
    pub total: u64,
    /// `MemAvailable`: free memory plus the cache Linux can drop.
    pub available: u64,
    /// Memory in the balloon.
    pub balloon: u64,
    /// Whether the guest takes memory back from the balloon when it runs out.
    pub deflate_on_oom: bool,
    /// Whether the guest zeroes pages as it allocates them (`init_on_alloc`).
    pub zeroes_pages: bool,
}

impl Stats {
    pub fn parse(text: &str) -> Option<Stats> {
        let (mut total, mut available) = (None, None);
        let (mut inflated, mut deflated, mut page, mut deflate_on_oom, mut zeroes_pages) = (0u64, 0u64, 4096u64, false, true);
        for line in text.lines() {
            let mut words = line.split_whitespace();
            let (Some(key), Some(value)) = (words.next(), words.next()) else { continue };
            let n = value.parse::<u64>().ok();
            match key {
                "MemTotal:" => total = n.map(|kb| kb * 1024),
                "MemAvailable:" => available = n.map(|kb| kb * 1024),
                "balloon_inflate" => inflated = n.unwrap_or(0),
                "balloon_deflate" => deflated = n.unwrap_or(0),
                "PageSize:" => page = n.unwrap_or(page),
                // Feature bits, lowest first: bit 2 is DEFLATE_ON_OOM.
                "BalloonFeatures:" => deflate_on_oom = value.as_bytes().get(2) == Some(&b'1'),
                "InitOnAlloc:" => zeroes_pages = value != "0",
                _ => {}
            }
        }
        let balloon = inflated.saturating_sub(deflated) * page;
        Some(Stats { total: total?, available: available?, balloon, deflate_on_oom, zeroes_pages })
    }

    /// What programs and the kernel use: not the cache, and not the balloon.
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available).saturating_sub(self.balloon)
    }
}

/// Where the balloon should be, from one reading to the next.
pub struct Balloon {
    /// The VM's memory size.
    size: u64,
    /// The memory the guest is meant to have now.
    target: u64,
    /// The most the guest wanted since `since`.
    peak: u64,
    since: Instant,
}

impl Balloon {
    pub fn new(size: u64, now: Instant) -> Balloon {
        Balloon { size, target: size, peak: 0, since: now }
    }

    /// What the guest should have: what it uses, room to grow, and what the
    /// kernel keeps for itself outside `MemTotal`.
    fn want(&self, s: &Stats) -> u64 {
        if !s.deflate_on_oom || s.zeroes_pages {
            return self.size;
        }
        let used = s.used();
        let reserved = self.size.saturating_sub(s.total);
        round_up(reserved + used + (used / 4).max(ROOM)).min(self.size)
    }

    /// The new target after a reading, when it changes.
    pub fn next(&mut self, s: &Stats, now: Instant) -> Option<u64> {
        let want = self.want(s);
        if want > self.target {
            // More room on top, so a guest that keeps growing isn't held to
            // one reading's worth at a time.
            self.target = round_up(want + ROOM).min(self.size);
            self.peak = want;
            self.since = now;
            return Some(self.target);
        }
        self.peak = self.peak.max(want);
        if now.duration_since(self.since) < SETTLE {
            return None;
        }
        let peak = std::mem::take(&mut self.peak);
        self.since = now;
        if peak + STEP > self.target {
            return None;
        }
        self.target = peak;
        Some(peak)
    }
}

fn round_up(n: u64) -> u64 {
    n.div_ceil(STEP) * STEP
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;

    /// A 4 GB guest using `used` MB, with `balloon` MB in the balloon.
    fn stats(used: u64, balloon: u64) -> Stats {
        let total = 4 * GB - 180 * MB;
        Stats { total, available: total - used * MB - balloon * MB, balloon: balloon * MB, deflate_on_oom: true, zeroes_pages: false }
    }

    #[test]
    fn parses_what_the_guest_reports() {
        let text = "MemTotal:        4011272 kB\nMemAvailable:     507588 kB\nballoon_inflate 300\nballoon_deflate 44\n\
                    balloon_migrate 0\nPageSize: 4096\nBalloonFeatures: 1010000000000000000000000000010010100000000000000000000000000000\n\
                    InitOnAlloc: 0\n";
        let s = Stats::parse(text).unwrap();
        assert_eq!(s.total, 4011272 * 1024);
        assert_eq!(s.available, 507588 * 1024);
        assert_eq!(s.balloon, 256 * 4096);
        assert!(s.deflate_on_oom);
        assert!(!s.zeroes_pages);
        assert_eq!(s.used(), (4011272 - 507588) * 1024 - 256 * 4096);
    }

    #[test]
    fn needs_both_memory_lines() {
        assert_eq!(Stats::parse("MemTotal: 100 kB\n"), None);
        assert_eq!(Stats::parse(""), None);
        let s = Stats::parse("MemTotal: 100 kB\nMemAvailable: 50 kB\nBalloonFeatures: 1000\n").unwrap();
        assert!(!s.deflate_on_oom);
        // A guest that doesn't say is taken to zero pages, as Debian does.
        assert!(s.zeroes_pages);
        assert_eq!(s.balloon, 0);
    }

    #[test]
    fn an_idle_guest_gives_memory_back_after_it_settles() {
        let t = Instant::now();
        let mut b = Balloon::new(4 * GB, t);
        assert_eq!(b.next(&stats(400, 0), t + INTERVAL), None);
        assert_eq!(b.next(&stats(400, 0), t + SETTLE - INTERVAL), None);
        // 180 MB the kernel keeps, 400 MB used, 1 GB of room: 1.58 GB, rounded up.
        assert_eq!(b.next(&stats(400, 0), t + SETTLE), Some(1664 * MB));
        assert_eq!(b.target, 1664 * MB);
        // Once the balloon holds the rest, the guest's use reads the same.
        assert_eq!(b.next(&stats(400, 2432), t + SETTLE * 2), None);
    }

    #[test]
    fn a_growing_guest_gets_memory_at_once() {
        let t = Instant::now();
        let mut b = Balloon::new(4 * GB, t);
        b.next(&stats(400, 0), t + SETTLE);
        assert_eq!(b.next(&stats(1000, 2432), t + SETTLE + INTERVAL), Some(3328 * MB));
        // It never gets more than the VM has.
        assert_eq!(b.next(&stats(3000, 0), t + SETTLE + INTERVAL * 2), Some(4 * GB));
    }

    #[test]
    fn a_spike_while_settling_keeps_its_memory() {
        let t = Instant::now();
        let mut b = Balloon::new(4 * GB, t);
        b.next(&stats(1500, 0), t + INTERVAL);
        b.next(&stats(400, 0), t + INTERVAL * 2);
        // The most it wanted in the last SETTLE: 180 + 1500 + 1024 MB, rounded up.
        assert_eq!(b.next(&stats(400, 0), t + SETTLE), Some(2816 * MB));
        // The next window starts over and lets it go.
        assert_eq!(b.next(&stats(400, 1280), t + SETTLE * 2), Some(1664 * MB));
    }

    #[test]
    fn small_changes_leave_the_balloon_alone() {
        let t = Instant::now();
        let mut b = Balloon::new(4 * GB, t);
        b.next(&stats(400, 0), t + SETTLE);
        assert_eq!(b.next(&stats(350, 2432), t + SETTLE * 2), None);
        assert_eq!(b.target, 1664 * MB);
    }

    #[test]
    fn a_guest_that_cant_take_memory_back_keeps_it_all() {
        let t = Instant::now();
        let mut b = Balloon::new(4 * GB, t);
        let s = Stats { deflate_on_oom: false, ..stats(400, 0) };
        assert_eq!(b.next(&s, t + SETTLE), None);
        assert_eq!(b.next(&s, t + SETTLE * 3), None);
        assert_eq!(b.target, 4 * GB);
    }

    #[test]
    fn a_guest_that_zeroes_pages_keeps_it_all() {
        let t = Instant::now();
        let mut b = Balloon::new(4 * GB, t);
        let s = Stats { zeroes_pages: true, ..stats(400, 0) };
        assert_eq!(b.next(&s, t + SETTLE), None);
        assert_eq!(b.target, 4 * GB);
    }

    #[test]
    fn a_small_vm_keeps_all_its_memory() {
        let t = Instant::now();
        let mut b = Balloon::new(GB, t);
        let s = Stats { total: GB - 60 * MB, available: GB - 260 * MB, balloon: 0, deflate_on_oom: true, zeroes_pages: false };
        assert_eq!(b.next(&s, t + SETTLE), None);
        assert_eq!(b.target, GB);
    }
}
