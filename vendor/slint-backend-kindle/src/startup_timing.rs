//! Opt-in monotonic startup markers for matched device measurements.

use std::{
    io::Write,
    sync::{
        OnceLock,
        atomic::{AtomicU8, Ordering},
    },
};

static ENABLED: OnceLock<bool> = OnceLock::new();
static FIRST_FRAME: AtomicU8 = AtomicU8::new(0);

fn enabled() -> bool {
    *ENABLED.get_or_init(|| std::env::var("WEREAD_STARTUP_TIMING").ok().as_deref() == Some("1"))
}

/// Emit only fixed stage names and a device-local monotonic timestamp.
/// Disabled ordinary launches perform no clock query or diagnostic write.
pub fn mark(stage: &'static str) {
    if !enabled() {
        return;
    }
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: time is the complete writable timespec used by CLOCK_MONOTONIC.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } == 0 {
        let _ = writeln!(
            std::io::stderr().lock(),
            "WEREAD_STARTUP {}.{:09} {stage}",
            time.tv_sec,
            time.tv_nsec
        );
    }
}

pub(crate) fn arm_first_application_frame() {
    if enabled() {
        let _ = FIRST_FRAME.compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed);
    }
}

pub(crate) fn update_submitted() {
    if FIRST_FRAME
        .compare_exchange(1, 2, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        mark("first_application_frame_send");
    }
}
