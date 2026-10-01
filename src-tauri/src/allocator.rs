//! Process-wide allocator and the timer that hands freed commit back.

// The default Windows system heap holds onto freed commit when a workload is
// bursty and fragmented - which is exactly the host process's job of buffering
// heavy PTY output (large base64 chunks) on its way to the webview. A single
// flood (a dev server, an AI CLI redrawing, a big build) inflates the commit to
// several GB and the heap never gives it back, so the process "stays at ~1 GB"
// long after the burst (a high-watermark, not a live leak: trimming the working
// set drops resident pages to a few MB while the committed bytes stay put).
// mimalloc purges freed segments back to the OS on a timer, so the watermark
// recedes once the burst ends.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// How often [`purge_allocator`] runs. Long enough that the sweep is free in
/// aggregate, short enough that a burst is handed back while the user is still
/// in the same sitting.
const ALLOCATOR_PURGE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// Hand freed-but-still-committed pages back to the OS.
///
/// Swapping the system heap for mimalloc (v0.3.76) was supposed to make the
/// commit recede on its own, and measurement says it does not: on a normal
/// working session the GUI host went 1.1 GB at 3 min, 7.0 GB at 20 min and
/// 15.8 GB at 64 min, while its working set stayed under 6 GB and dropped to
/// 24 MB under `EmptyWorkingSet` without climbing back. So the bulk of it was
/// committed, freed, untouched, and still charged against the system commit
/// limit, which was already at 32.8 of 44.1 GB. mimalloc only purges a segment
/// that happens to fall entirely free, and a bursty producer (the PTY -> webview
/// path buffers large base64 chunks) strands committed pages across many
/// partly-used segments, so the process commit only ever ratchets up.
///
/// `mi_collect(true)` forces the sweep that the timer-driven purge misses.
/// `libmimalloc-sys` at this version exposes no `mi_option_purge_delay`
/// constant, so this is the available lever; see the test at the bottom of this
/// file for the measured before/after.
pub fn purge_allocator() {
    // SAFETY: `mi_collect` takes no pointers, is documented thread-safe, and is
    // a no-op when there is nothing to reclaim.
    unsafe { libmimalloc_sys::mi_collect(true) };
}

/// Run [`purge_allocator`] on a timer, off the UI thread. Deliberately its own
/// thread rather than a Tauri async task: the sweep walks the heap, and the one
/// thing it must never do is share a thread with anything that draws.
pub(crate) fn spawn_allocator_purge_thread() {
    let _ = std::thread::Builder::new()
        .name("subclave-alloc-purge".into())
        .spawn(|| loop {
            std::thread::sleep(ALLOCATOR_PURGE_INTERVAL);
            purge_allocator();
        });
}

#[cfg(all(test, target_os = "windows"))]
mod allocator_tests {
    use super::purge_allocator;

    /// Bytes this process has committed privately, i.e. what it charges against
    /// the system commit limit. Deliberately NOT the working set: the whole
    /// point of the bug is that the working set looks fine while the commit
    /// climbs into double-digit GB.
    fn committed_private_bytes() -> usize {
        use windows::Win32::System::Memory::{
            VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, MEM_PRIVATE,
        };
        let stride = std::mem::size_of::<MEMORY_BASIC_INFORMATION>();
        let mut addr: usize = 0;
        let mut total: usize = 0;
        loop {
            let mut mbi = MEMORY_BASIC_INFORMATION::default();
            // SAFETY: querying our own address space; `mbi` is a live, correctly
            // sized buffer. A zero return means the walk ran off the top.
            let read = unsafe { VirtualQuery(Some(addr as *const _), &mut mbi, stride) };
            if read == 0 {
                break;
            }
            if mbi.State == MEM_COMMIT && mbi.Type == MEM_PRIVATE {
                total += mbi.RegionSize;
            }
            let next = mbi.BaseAddress as usize + mbi.RegionSize;
            if next <= addr {
                break; // no forward progress; refuse to spin
            }
            addr = next;
        }
        total
    }

    const CHUNK: usize = 1024 * 1024;

    /// Allocate `chunks` MiB, touch every page, then drop it all. `vec![_; _]`
    /// memsets, so this is committed AND resident, exactly like a base64 chunk
    /// on its way to the webview.
    fn burst_and_free(chunks: usize) {
        let held: Vec<Vec<u8>> = (0..chunks).map(|_| vec![7u8; CHUNK]).collect();
        std::hint::black_box(&held);
    }

    /// The regression guard for "Subclave gets heavy and then stops responding after
    /// a long session". Both halves are needed and they assert different things.
    ///
    /// Deliberately ONE test function rather than two: each phase reads the
    /// process-wide commit, and `cargo test` runs test functions in parallel, so
    /// two allocator tests would measure each other's bursts and flake.
    #[test]
    fn freed_memory_goes_back_to_the_os_and_does_not_ratchet() {
        // Phase 1: a single burst must not stay charged to the process. Measured
        // without the fix, the allocator returned NOTHING: 1090 MiB of a 1090 MiB
        // burst was still committed afterwards.
        let base = committed_private_bytes();
        let held: Vec<Vec<u8>> = (0..1024).map(|_| vec![7u8; CHUNK]).collect();
        let peak = committed_private_bytes();
        drop(held);
        purge_allocator();
        let after = committed_private_bytes();

        let grew = peak.saturating_sub(base);
        let kept = after.saturating_sub(base);
        assert!(
            grew > 512 * 1024 * 1024,
            "burst did not register: base={base} peak={peak} (grew {grew})"
        );
        assert!(
            kept * 4 < grew,
            "allocator kept {} MiB of the {} MiB burst after purge_allocator()",
            kept / 1024 / 1024,
            grew / 1024 / 1024
        );

        // Phase 2: the actual complaint is not that one burst is expensive, it is
        // that an hour of them never comes back down. Repeated cycles must land
        // in the same place instead of drifting upward, which is the "long
        // session stays flat" property in miniature.
        let mut marks = Vec::new();
        for _ in 0..3 {
            burst_and_free(256);
            purge_allocator();
            marks.push(committed_private_bytes());
        }
        let drift = marks.last().unwrap().saturating_sub(marks[0]);
        assert!(
            drift < 64 * 1024 * 1024,
            "commit drifted up {} MiB across three burst cycles: {:?} MiB",
            drift / 1024 / 1024,
            marks.iter().map(|m| m / 1024 / 1024).collect::<Vec<_>>()
        );
    }
}
