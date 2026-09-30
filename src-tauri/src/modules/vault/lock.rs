//! Idle lock on a boot-time clock.
//!
//! Wall-clock time can be set back by the user or an NTP step, which would
//! extend an idle deadline; a boot-time clock counts sleep and never steps
//! backwards. The deadline is checked on a 5 s tick and before every vault
//! command, so a machine waking past the deadline locks before the first
//! command runs.

/// Milliseconds on a clock that counts sleep and never steps backwards:
/// `CLOCK_BOOTTIME` on Linux, `CLOCK_MONOTONIC` on macOS (Apple's monotonic
/// constant counts suspend; the libc 0.2.186 bindings carry no
/// `mach_continuous_time`), `GetTickCount64` on Windows.
pub fn boot_now_ms() -> u64 {
    #[cfg(target_os = "linux")]
    {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: plain clock read into a stack timespec; no allocation.
        let rc = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) };
        debug_assert_eq!(rc, 0, "clock_gettime(CLOCK_BOOTTIME) failed");
        (ts.tv_sec.max(0) as u64) * 1_000 + (ts.tv_nsec.max(0) as u64) / 1_000_000
    }
    #[cfg(target_os = "macos")]
    {
        let mut ts = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: plain clock read into a stack timespec; no allocation.
        let rc = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
        debug_assert_eq!(rc, 0, "clock_gettime(CLOCK_MONOTONIC) failed");
        (ts.tv_sec.max(0) as u64) * 1_000 + (ts.tv_nsec.max(0) as u64) / 1_000_000
    }
    #[cfg(target_os = "windows")]
    {
        // SAFETY: parameterless Win32 call.
        unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() }
    }
}

/// `minutes == 0` means never: the sentinel is `u64::MAX`. Saturating
/// arithmetic because the input comes from settings, not from a constant.
pub fn deadline_after(minutes: u64, now: u64) -> u64 {
    if minutes == 0 {
        u64::MAX
    } else {
        now.saturating_add(minutes.saturating_mul(60_000))
    }
}

/// `u64::MAX` is the "never" sentinel and never passes.
pub fn deadline_passed(now: u64, deadline: u64) -> bool {
    now >= deadline && deadline != u64::MAX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_clock_is_monotonic_across_reads() {
        let a = boot_now_ms();
        let b = boot_now_ms();
        assert!(b >= a, "{b} < {a}");
        assert!(a > 0, "a boot clock is past 0 by the time any process runs");
    }

    #[test]
    fn zero_minutes_is_the_never_sentinel() {
        assert_eq!(deadline_after(0, 1_000), u64::MAX);
        assert!(!deadline_passed(u64::MAX, u64::MAX));
    }

    #[test]
    fn deadline_math_saturates() {
        assert_eq!(deadline_after(10, 5_000), 605_000);
        assert_eq!(deadline_after(u64::MAX / 1_000, u64::MAX - 1), u64::MAX);
    }

    #[test]
    fn expiry_at_and_past_the_deadline() {
        assert!(!deadline_passed(59_999, 60_000));
        assert!(deadline_passed(60_000, 60_000));
        assert!(deadline_passed(61_000, 60_000));
    }
}
